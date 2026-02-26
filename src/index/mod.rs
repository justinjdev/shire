pub mod cargo;
pub mod go;
pub mod go_work;
pub mod hash;
pub mod manifest;
pub mod npm;
pub mod python;

use crate::config::Config;
use crate::db;
use crate::symbols;
use anyhow::Result;
use ignore::WalkBuilder;
use manifest::{ManifestParser, PackageInfo};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A discovered manifest file with its relative dir and content hash.
struct WalkedManifest {
    abs_path: PathBuf,
    relative_dir: String,
    /// Relative manifest path used as DB key (e.g. "services/auth/package.json")
    manifest_key: String,
    content_hash: String,
}

/// Walk the repo and collect manifest paths with content hashes.
fn walk_manifests(
    repo_root: &Path,
    config: &Config,
    parsers: &[Box<dyn ManifestParser>],
) -> Result<Vec<WalkedManifest>> {
    let mut manifest_filenames: HashSet<&str> = parsers.iter().map(|p| p.filename()).collect();
    // go.work provides workspace context, not packages — but must be walked
    manifest_filenames.insert("go.work");
    let enabled: HashSet<&str> = config
        .discovery
        .manifests
        .iter()
        .map(|s| s.as_str())
        .collect();
    let exclude_set: HashSet<String> = config.discovery.exclude.iter().cloned().collect();

    let walker = WalkBuilder::new(repo_root)
        .hidden(true)
        .filter_entry(move |entry| {
            if let Some(name) = entry.file_name().to_str() {
                if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                    return !exclude_set.contains(name);
                }
            }
            true
        })
        .build();

    let mut manifests = Vec::new();

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().map_or(false, |ft| ft.is_file()) {
            continue;
        }

        let filename = match entry.file_name().to_str() {
            Some(f) => f.to_string(),
            None => continue,
        };

        if !manifest_filenames.contains(filename.as_str())
            || !enabled.contains(filename.as_str())
        {
            continue;
        }

        let file_path = entry.into_path();
        let relative_dir = file_path
            .parent()
            .and_then(|p| p.strip_prefix(repo_root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let manifest_key = if relative_dir.is_empty() {
            filename.clone()
        } else {
            format!("{}/{}", relative_dir, filename)
        };

        let content_hash = hash::hash_file(&file_path)?;

        manifests.push(WalkedManifest {
            abs_path: file_path,
            relative_dir,
            manifest_key,
            content_hash,
        });
    }

    Ok(manifests)
}

/// Load stored manifest hashes from the DB.
fn load_stored_hashes(conn: &Connection) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT path, content_hash FROM manifest_hashes")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (path, hash) = row?;
        map.insert(path, hash);
    }
    Ok(map)
}

/// Diff walked manifests against stored hashes.
struct ManifestDiff<'a> {
    new: Vec<&'a WalkedManifest>,
    changed: Vec<&'a WalkedManifest>,
    unchanged: Vec<&'a WalkedManifest>,
    removed: Vec<String>, // manifest keys no longer on disk
}

fn diff_manifests<'a>(
    walked: &'a [WalkedManifest],
    stored: &HashMap<String, String>,
) -> ManifestDiff<'a> {
    let mut new = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();

    let walked_keys: HashSet<&str> = walked.iter().map(|m| m.manifest_key.as_str()).collect();

    for manifest in walked {
        match stored.get(&manifest.manifest_key) {
            None => new.push(manifest),
            Some(old_hash) if *old_hash != manifest.content_hash => changed.push(manifest),
            Some(_) => unchanged.push(manifest),
        }
    }

    let removed: Vec<String> = stored
        .keys()
        .filter(|k| !walked_keys.contains(k.as_str()))
        .cloned()
        .collect();

    ManifestDiff {
        new,
        changed,
        unchanged,
        removed,
    }
}

/// Insert a package and its dependencies into the DB.
fn upsert_package(conn: &Connection, pkg: &PackageInfo) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO packages (name, path, kind, version, description, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            &pkg.name,
            &pkg.path,
            pkg.kind,
            &pkg.version,
            &pkg.description,
            &pkg.metadata.as_ref().map(|m| m.to_string()),
        ),
    )?;

    // Clear old deps before inserting new ones
    conn.execute("DELETE FROM dependencies WHERE package = ?1", [&pkg.name])?;

    let mut dep_stmt = conn.prepare(
        "INSERT OR REPLACE INTO dependencies (package, dependency, dep_kind, version_req, is_internal)
         VALUES (?1, ?2, ?3, ?4, 0)",
    )?;
    for dep in &pkg.dependencies {
        dep_stmt.execute((&pkg.name, &dep.name, dep.dep_kind.as_str(), &dep.version_req))?;
    }
    Ok(())
}

/// Recompute is_internal for all dependencies using a single SQL UPDATE.
/// Handles both direct package name matches and Go module path aliases.
fn recompute_is_internal(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE dependencies SET is_internal = (
            dependency IN (SELECT name FROM packages)
            OR dependency IN (SELECT description FROM packages WHERE kind = 'go' AND description IS NOT NULL)
        )",
        [],
    )?;
    Ok(())
}

/// Clear and re-insert symbols for a package.
fn upsert_symbols(conn: &Connection, package: &str, syms: &[symbols::SymbolInfo]) -> Result<()> {
    conn.execute("DELETE FROM symbols WHERE package = ?1", [package])?;

    let mut stmt = conn.prepare(
        "INSERT INTO symbols (package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, parameters)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;

    for sym in syms {
        let params_json = sym
            .parameters
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_default());

        stmt.execute((
            package,
            &sym.name,
            sym.kind.as_str(),
            &sym.signature,
            &sym.file_path,
            sym.line as i64,
            &sym.visibility,
            &sym.parent_symbol,
            &sym.return_type,
            &params_json,
        ))?;
    }

    Ok(())
}

/// Store or update a source hash for a package.
fn upsert_source_hash(conn: &Connection, package: &str, hash: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO source_hashes (package, content_hash) VALUES (?1, ?2)",
        (package, hash),
    )?;
    Ok(())
}

/// A discovered file during file walking.
struct WalkedFile {
    relative_path: String,
    extension: String,
    size_bytes: u64,
}

/// Walk the repo and collect all files with metadata.
fn walk_files(repo_root: &Path, config: &Config) -> Result<Vec<WalkedFile>> {
    let exclude_set: HashSet<String> = config.discovery.exclude.iter().cloned().collect();

    let walker = WalkBuilder::new(repo_root)
        .hidden(true)
        .filter_entry(move |entry| {
            if let Some(name) = entry.file_name().to_str() {
                if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                    return !exclude_set.contains(name);
                }
            }
            true
        })
        .build();

    let mut files = Vec::new();

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().map_or(false, |ft| ft.is_file()) {
            continue;
        }

        let file_path = entry.path();
        let relative_path = file_path
            .strip_prefix(repo_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);

        files.push(WalkedFile {
            relative_path,
            extension,
            size_bytes,
        });
    }

    Ok(files)
}

/// Associate files with their owning package using longest-prefix matching.
fn associate_files_with_packages(
    files: &[WalkedFile],
    packages: &[(String, String)], // (name, path)
) -> Vec<(String, Option<String>, String, u64)> {
    // Sort package paths by length descending so longest prefix matches first
    let mut sorted_pkgs: Vec<&(String, String)> = packages.iter().collect();
    sorted_pkgs.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    files
        .iter()
        .map(|file| {
            let file_dir = file
                .relative_path
                .rsplit_once('/')
                .map(|(dir, _)| dir)
                .unwrap_or("");

            let package = sorted_pkgs.iter().find_map(|(name, path)| {
                if path.is_empty() {
                    // Root-level package matches everything
                    Some(name.clone())
                } else if file_dir == path.as_str() || file_dir.starts_with(&format!("{}/", path)) {
                    Some(name.clone())
                } else {
                    None
                }
            });

            (
                file.relative_path.clone(),
                package,
                file.extension.clone(),
                file.size_bytes,
            )
        })
        .collect()
}

/// Clear and re-insert all files.
fn upsert_files(
    conn: &Connection,
    files: &[(String, Option<String>, String, u64)],
) -> Result<()> {
    conn.execute("DELETE FROM files", [])?;
    let mut stmt = conn.prepare(
        "INSERT INTO files (path, package, extension, size_bytes) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (path, package, ext, size) in files {
        stmt.execute((path, package, ext, *size as i64))?;
    }
    Ok(())
}

/// Scan walked Cargo.toml files for workspace roots and collect `[workspace.dependencies]`.
fn collect_cargo_workspace_context(walked: &[WalkedManifest]) -> HashMap<String, String> {
    let mut all_ws_deps = HashMap::new();

    for manifest in walked {
        let filename = manifest
            .abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        if filename == "Cargo.toml" {
            if let Ok(deps) = cargo::collect_cargo_workspace_deps(&manifest.abs_path) {
                all_ws_deps.extend(deps);
            }
        }
    }

    all_ws_deps
}

/// Scan walked go.work files and collect the set of workspace member directories.
fn collect_go_workspace_context(walked: &[WalkedManifest]) -> HashSet<String> {
    let mut dirs = HashSet::new();

    for manifest in walked {
        let filename = manifest
            .abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        if filename == "go.work" {
            if let Ok(use_dirs) = go_work::parse_go_work(&manifest.abs_path) {
                for d in use_dirs {
                    // go.work use directives are relative to the go.work location
                    let full_dir = if manifest.relative_dir.is_empty() {
                        d
                    } else {
                        format!("{}/{}", manifest.relative_dir, d)
                    };
                    dirs.insert(full_dir);
                }
            }
        }
    }

    dirs
}

pub fn build_index(repo_root: &Path, config: &Config, force: bool) -> Result<()> {
    let db_dir = repo_root.join(".shire");
    let db_path = db_dir.join("index.db");
    let conn = db::open_or_create(&db_path)?;

    // If --force, clear hashes and symbols to trigger full rebuild
    if force {
        conn.execute("DELETE FROM manifest_hashes", [])?;
        conn.execute("DELETE FROM symbols", [])?;
        conn.execute("DELETE FROM source_hashes", [])?;
    }

    let parsers: Vec<Box<dyn ManifestParser>> = vec![
        Box::new(npm::NpmParser),
        Box::new(go::GoParser),
        Box::new(cargo::CargoParser),
        Box::new(python::PythonParser),
    ];

    // Phase 1: Walk — discover all manifests with content hashes
    let walked = walk_manifests(repo_root, config, &parsers)?;

    // Phase 1.5: Collect workspace context before parsing
    let cargo_workspace_deps = collect_cargo_workspace_context(&walked);
    let go_workspace_dirs = collect_go_workspace_context(&walked);

    // Phase 2: Diff — compare against stored hashes
    let stored_hashes = load_stored_hashes(&conn)?;
    let diff = diff_manifests(&walked, &stored_hashes);

    let is_full_build = stored_hashes.is_empty();
    let to_parse: Vec<&WalkedManifest> = diff
        .new
        .iter()
        .chain(diff.changed.iter())
        .copied()
        .collect();

    let num_added = diff.new.len();
    let num_changed = diff.changed.len();
    let num_removed = diff.removed.len();
    let num_skipped = diff.unchanged.len();

    // Phase 3: Parse — only new + changed manifests
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut parsed_packages: Vec<(String, String, String)> = Vec::new(); // (name, path, kind)
    let cargo_parser = cargo::CargoParser;

    for manifest in &to_parse {
        let filename = manifest
            .abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        // Skip go.work files — they provide context, not packages
        if filename == "go.work" {
            continue;
        }

        // Cargo members: use workspace-aware parsing when context exists
        if filename == "Cargo.toml" && !cargo_workspace_deps.is_empty() {
            match cargo_parser.parse_with_workspace_deps(
                &manifest.abs_path,
                &manifest.relative_dir,
                &cargo_workspace_deps,
            ) {
                Ok(pkg) => {
                    parsed_packages.push((pkg.name.clone(), pkg.path.clone(), pkg.kind.to_string()));
                    upsert_package(&conn, &pkg)?;
                }
                Err(e) => {
                    failures.push((manifest.abs_path.display().to_string(), e.to_string()));
                }
            }
            continue;
        }

        for parser in &parsers {
            if parser.filename() == filename {
                match parser.parse(&manifest.abs_path, &manifest.relative_dir) {
                    Ok(mut pkg) => {
                        // Annotate Go packages that are go.work members
                        if pkg.kind == "go" && go_workspace_dirs.contains(&manifest.relative_dir) {
                            pkg.metadata = Some(serde_json::json!({"go_workspace": true}));
                        }
                        parsed_packages.push((pkg.name.clone(), pkg.path.clone(), pkg.kind.to_string()));
                        upsert_package(&conn, &pkg)?;
                    }
                    Err(e) => {
                        failures.push((manifest.abs_path.display().to_string(), e.to_string()));
                    }
                }
                break;
            }
        }
    }

    // Phase 4: Remove packages from deleted manifests
    for manifest_key in &diff.removed {
        // Look up which package came from this path
        let relative_dir = manifest_key
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        conn.execute(
            "DELETE FROM source_hashes WHERE package IN (SELECT name FROM packages WHERE path = ?1)",
            [relative_dir],
        )?;
        conn.execute(
            "DELETE FROM symbols WHERE package IN (SELECT name FROM packages WHERE path = ?1)",
            [relative_dir],
        )?;
        conn.execute(
            "DELETE FROM dependencies WHERE package IN (SELECT name FROM packages WHERE path = ?1)",
            [relative_dir],
        )?;
        conn.execute("DELETE FROM packages WHERE path = ?1", [relative_dir])?;
        conn.execute(
            "DELETE FROM manifest_hashes WHERE path = ?1",
            [manifest_key.as_str()],
        )?;
    }

    // Phase 5: Recompute is_internal for ALL deps
    if num_added > 0 || num_changed > 0 || num_removed > 0 {
        recompute_is_internal(&conn)?;
    }

    // Phase 6: Update stored hashes for new + changed manifests
    let mut hash_stmt = conn.prepare(
        "INSERT OR REPLACE INTO manifest_hashes (path, content_hash) VALUES (?1, ?2)",
    )?;
    for manifest in &to_parse {
        hash_stmt.execute((&manifest.manifest_key, &manifest.content_hash))?;
    }

    // Phase 7: Extract symbols for new/changed packages
    for (pkg_name, pkg_path, pkg_kind) in &parsed_packages {
        match symbols::extract_symbols_for_package(repo_root, pkg_path, pkg_kind) {
            Ok(syms) => {
                upsert_symbols(&conn, pkg_name, &syms)?;
            }
            Err(e) => {
                eprintln!("Warning: symbol extraction failed for {}: {}", pkg_name, e);
            }
        }
        // Store source hash for manifest-changed packages
        if let Ok(src_hash) = hash::compute_source_hash(repo_root, pkg_path, pkg_kind) {
            let _ = upsert_source_hash(&conn, pkg_name, &src_hash);
        }
    }

    // Phase 8: Source-level incremental — re-extract symbols for unchanged packages
    // whose source files have changed
    let mut num_source_reextracted: usize = 0;
    for manifest in &diff.unchanged {
        let relative_dir = &manifest.relative_dir;

        // Look up the package from DB by path
        let pkg_info: Option<(String, String)> = conn
            .query_row(
                "SELECT name, kind FROM packages WHERE path = ?1",
                [relative_dir.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let (pkg_name, pkg_kind) = match pkg_info {
            Some(info) => info,
            None => continue, // no package for this manifest (e.g., go.work)
        };

        // Compute current source hash
        let current_hash = match hash::compute_source_hash(repo_root, relative_dir, &pkg_kind) {
            Ok(h) => h,
            Err(_) => continue,
        };

        // Load stored source hash
        let stored_hash: Option<String> = conn
            .query_row(
                "SELECT content_hash FROM source_hashes WHERE package = ?1",
                [&pkg_name],
                |row| row.get(0),
            )
            .ok();

        // Re-extract if hash differs or no stored hash
        if stored_hash.as_deref() != Some(&current_hash) {
            match symbols::extract_symbols_for_package(repo_root, relative_dir, &pkg_kind) {
                Ok(syms) => {
                    upsert_symbols(&conn, &pkg_name, &syms)?;
                    num_source_reextracted += 1;
                }
                Err(e) => {
                    eprintln!("Warning: symbol re-extraction failed for {}: {}", pkg_name, e);
                }
            }
            let _ = upsert_source_hash(&conn, &pkg_name, &current_hash);
        }
    }

    // Phase 9: Index files — walk all files, associate with packages, insert
    let all_packages: Vec<(String, String)> = conn
        .prepare("SELECT name, path FROM packages")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let walked_files = walk_files(repo_root, config)?;
    let associated_files = associate_files_with_packages(&walked_files, &all_packages);
    let num_files = associated_files.len();
    upsert_files(&conn, &associated_files)?;

    // Apply config overrides
    for override_pkg in &config.packages {
        if let Some(desc) = &override_pkg.description {
            let rows = conn.execute(
                "UPDATE packages SET description = ?1 WHERE name = ?2",
                (desc, &override_pkg.name),
            )?;
            if rows == 0 {
                eprintln!(
                    "Warning: config override for '{}' matched no packages",
                    override_pkg.name
                );
            }
        }
    }

    // Store metadata
    let git_commit = match std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                eprintln!("Note: git rev-parse failed (not a git repo?)");
                None
            }
        }
        Err(e) => {
            eprintln!("Warning: could not run git: {}", e);
            None
        }
    };

    let total_packages: i64 = conn.query_row(
        "SELECT COUNT(*) FROM packages",
        [],
        |row| row.get(0),
    )?;
    let total_symbols: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols",
        [],
        |row| row.get(0),
    )?;

    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('indexed_at', ?1)",
        [chrono::Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('package_count', ?1)",
        [total_packages.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('symbol_count', ?1)",
        [total_symbols.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('file_count', ?1)",
        [num_files.to_string()],
    )?;
    if let Some(commit) = git_commit {
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('git_commit', ?1)",
            [commit],
        )?;
    }

    if !failures.is_empty() {
        eprintln!("{} manifest(s) failed to parse:", failures.len());
        for (path, err) in &failures {
            eprintln!("  {}: {}", path, err);
        }
    }

    if is_full_build || force {
        println!(
            "Indexed {} packages, {} symbols, {} files into {}",
            total_packages, total_symbols, num_files,
            db_path.display()
        );
    } else if num_source_reextracted > 0 {
        println!(
            "Indexed {} packages ({} added, {} updated, {} removed, {} skipped, {} source-updated), {} symbols, {} files into {}",
            total_packages, num_added, num_changed, num_removed, num_skipped, num_source_reextracted, total_symbols, num_files,
            db_path.display()
        );
    } else {
        println!(
            "Indexed {} packages ({} added, {} updated, {} removed, {} skipped), {} symbols, {} files into {}",
            total_packages, num_added, num_changed, num_removed, num_skipped, total_symbols, num_files,
            db_path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn create_test_monorepo(dir: &Path) {
        // npm package
        let npm_dir = dir.join("services/auth");
        fs::create_dir_all(&npm_dir).unwrap();
        let mut f = fs::File::create(npm_dir.join("package.json")).unwrap();
        f.write_all(
            br#"{"name": "auth-service", "version": "1.0.0", "description": "Auth", "dependencies": {"shared-types": "^1.0"}}"#,
        ).unwrap();

        // Another npm package (the dependency)
        let shared_dir = dir.join("packages/shared-types");
        fs::create_dir_all(&shared_dir).unwrap();
        let mut f = fs::File::create(shared_dir.join("package.json")).unwrap();
        f.write_all(
            br#"{"name": "shared-types", "version": "1.0.0", "description": "Shared TypeScript types"}"#,
        ).unwrap();

        // Go package
        let go_dir = dir.join("services/gateway");
        fs::create_dir_all(&go_dir).unwrap();
        let mut f = fs::File::create(go_dir.join("go.mod")).unwrap();
        f.write_all(b"module github.com/company/gateway\n\ngo 1.22\n").unwrap();
    }

    #[test]
    fn test_build_index_creates_db() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());

        let config = Config::default();
        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        assert!(db_path.exists());

        let conn = db::open_readonly(&db_path).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);

        // Check is_internal flag
        let is_internal: bool = conn
            .query_row(
                "SELECT is_internal FROM dependencies WHERE package='auth-service' AND dependency='shared-types'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(is_internal);
    }

    #[test]
    fn test_fts_search_works_after_build() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());

        let config = Config::default();
        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        let results: Vec<String> = conn
            .prepare("SELECT name FROM packages_fts WHERE packages_fts MATCH ?1")
            .unwrap()
            .query_map(["auth"], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(results.contains(&"auth-service".to_string()));
    }

    fn hash_count(dir: &Path) -> i64 {
        let db_path = dir.join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();
        conn.query_row("SELECT COUNT(*) FROM manifest_hashes", [], |row| row.get(0))
            .unwrap()
    }

    fn pkg_count(dir: &Path) -> i64 {
        let db_path = dir.join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();
        conn.query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn test_incremental_no_changes_skips_all() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        // First build
        build_index(dir.path(), &config, false).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);
        assert_eq!(hash_count(dir.path()), 3);

        // Second build — nothing changed
        build_index(dir.path(), &config, false).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);
    }

    #[test]
    fn test_incremental_modified_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        build_index(dir.path(), &config, false).unwrap();

        // Modify auth-service version
        let auth_path = dir.path().join("services/auth/package.json");
        fs::write(
            &auth_path,
            br#"{"name": "auth-service", "version": "2.0.0", "description": "Auth v2", "dependencies": {"shared-types": "^1.0"}}"#,
        ).unwrap();

        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();
        let version: String = conn
            .query_row(
                "SELECT version FROM packages WHERE name = 'auth-service'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "2.0.0");
        assert_eq!(pkg_count(dir.path()), 3);
    }

    #[test]
    fn test_incremental_deleted_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        build_index(dir.path(), &config, false).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);

        // Delete the Go package
        fs::remove_file(dir.path().join("services/gateway/go.mod")).unwrap();

        build_index(dir.path(), &config, false).unwrap();
        assert_eq!(pkg_count(dir.path()), 2);
        assert_eq!(hash_count(dir.path()), 2);
    }

    #[test]
    fn test_incremental_added_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        build_index(dir.path(), &config, false).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);

        // Add a new npm package
        let new_dir = dir.path().join("services/billing");
        fs::create_dir_all(&new_dir).unwrap();
        fs::write(
            new_dir.join("package.json"),
            br#"{"name": "billing", "version": "1.0.0", "description": "Billing service"}"#,
        ).unwrap();

        build_index(dir.path(), &config, false).unwrap();
        assert_eq!(pkg_count(dir.path()), 4);
        assert_eq!(hash_count(dir.path()), 4);
    }

    #[test]
    fn test_incremental_is_internal_updates_on_add() {
        let dir = tempfile::TempDir::new().unwrap();

        // Start with just auth-service depending on "billing" (external)
        let auth_dir = dir.path().join("services/auth");
        fs::create_dir_all(&auth_dir).unwrap();
        fs::write(
            auth_dir.join("package.json"),
            br#"{"name": "auth-service", "version": "1.0.0", "dependencies": {"billing": "^1.0"}}"#,
        ).unwrap();

        let config = Config::default();
        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();
        let is_internal: bool = conn
            .query_row(
                "SELECT is_internal FROM dependencies WHERE package='auth-service' AND dependency='billing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!is_internal);
        drop(conn);

        // Now add "billing" as an internal package
        let billing_dir = dir.path().join("services/billing");
        fs::create_dir_all(&billing_dir).unwrap();
        fs::write(
            billing_dir.join("package.json"),
            br#"{"name": "billing", "version": "1.0.0"}"#,
        ).unwrap();

        build_index(dir.path(), &config, false).unwrap();

        let conn = db::open_readonly(&db_path).unwrap();
        let is_internal: bool = conn
            .query_row(
                "SELECT is_internal FROM dependencies WHERE package='auth-service' AND dependency='billing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(is_internal);
    }

    #[test]
    fn test_force_rebuild() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        build_index(dir.path(), &config, false).unwrap();
        assert_eq!(hash_count(dir.path()), 3);

        // Force rebuild — should still work and produce same result
        build_index(dir.path(), &config, true).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);
        assert_eq!(hash_count(dir.path()), 3);
    }

    #[test]
    fn test_cargo_workspace_dep_resolution() {
        let dir = tempfile::TempDir::new().unwrap();

        // Workspace root Cargo.toml (no [package], has [workspace])
        let root = dir.path();
        fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/*"]

[workspace.dependencies]
tokio = { version = "1.35", features = ["full"] }
serde = "1.0"
"#,
        )
        .unwrap();

        // Member crate using workspace = true
        let member_dir = root.join("crates/my-service");
        fs::create_dir_all(&member_dir).unwrap();
        fs::write(
            member_dir.join("Cargo.toml"),
            r#"
[package]
name = "my-service"
version = "0.1.0"

[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
anyhow = "1"
"#,
        )
        .unwrap();

        let config = Config::default();
        build_index(root, &config, false).unwrap();

        let db_path = root.join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        // Verify workspace deps resolved
        let tokio_ver: Option<String> = conn
            .query_row(
                "SELECT version_req FROM dependencies WHERE package='my-service' AND dependency='tokio'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tokio_ver.as_deref(), Some("1.35"));

        let serde_ver: Option<String> = conn
            .query_row(
                "SELECT version_req FROM dependencies WHERE package='my-service' AND dependency='serde'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(serde_ver.as_deref(), Some("1.0"));

        // Non-workspace dep should have its own version
        let anyhow_ver: Option<String> = conn
            .query_row(
                "SELECT version_req FROM dependencies WHERE package='my-service' AND dependency='anyhow'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(anyhow_ver.as_deref(), Some("1"));

        // Only 1 package (member) — workspace root has no [package]
        assert_eq!(pkg_count(root), 1);
    }

    #[test]
    fn test_npm_workspace_protocol_in_index() {
        let dir = tempfile::TempDir::new().unwrap();

        let app_dir = dir.path().join("packages/app");
        fs::create_dir_all(&app_dir).unwrap();
        fs::write(
            app_dir.join("package.json"),
            br#"{"name": "app", "version": "1.0.0", "dependencies": {"shared": "workspace:*"}}"#,
        )
        .unwrap();

        let shared_dir = dir.path().join("packages/shared");
        fs::create_dir_all(&shared_dir).unwrap();
        fs::write(
            shared_dir.join("package.json"),
            br#"{"name": "shared", "version": "2.0.0"}"#,
        )
        .unwrap();

        let config = Config::default();
        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        let version_req: Option<String> = conn
            .query_row(
                "SELECT version_req FROM dependencies WHERE package='app' AND dependency='shared'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version_req.as_deref(), Some("*"));
    }

    #[test]
    fn test_go_work_metadata() {
        let dir = tempfile::TempDir::new().unwrap();

        // go.work at root
        fs::write(
            dir.path().join("go.work"),
            "go 1.22\n\nuse (\n\t./services/auth\n)\n",
        )
        .unwrap();

        // Go module that IS in the workspace
        let auth_dir = dir.path().join("services/auth");
        fs::create_dir_all(&auth_dir).unwrap();
        fs::write(
            auth_dir.join("go.mod"),
            "module github.com/company/auth\n\ngo 1.22\n",
        )
        .unwrap();

        // Go module that is NOT in the workspace
        let other_dir = dir.path().join("tools/cli");
        fs::create_dir_all(&other_dir).unwrap();
        fs::write(
            other_dir.join("go.mod"),
            "module github.com/company/cli\n\ngo 1.22\n",
        )
        .unwrap();

        let config = Config::default();
        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        // Auth should have go_workspace metadata
        let auth_meta: Option<String> = conn
            .query_row(
                "SELECT metadata FROM packages WHERE name = 'auth'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(auth_meta.is_some());
        let meta: serde_json::Value = serde_json::from_str(auth_meta.as_deref().unwrap()).unwrap();
        assert_eq!(meta["go_workspace"], true);

        // CLI tool should have no metadata
        let cli_meta: Option<String> = conn
            .query_row(
                "SELECT metadata FROM packages WHERE name = 'cli'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(cli_meta.is_none());
    }
}
