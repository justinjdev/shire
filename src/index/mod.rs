pub mod cargo;
pub mod go;
pub mod go_work;
pub mod gradle;
pub mod gradle_settings;
pub mod hash;
pub mod manifest;
pub mod maven;
pub mod npm;
pub mod python;

use crate::config::Config;
use crate::db;
use crate::symbols;
use anyhow::Result;
use ignore::WalkBuilder;
use manifest::{ManifestParser, PackageInfo};
use rayon::prelude::*;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Execute a closure within an explicit SQLite transaction.
/// Commits on success, rolls back on error.
fn with_transaction<F, T>(conn: &Connection, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    conn.execute_batch("BEGIN")?;
    match f() {
        Ok(val) => {
            conn.execute_batch("COMMIT")?;
            Ok(val)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// A discovered manifest file with its relative dir and content hash.
pub(crate) struct WalkedManifest {
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
    // settings.gradle provides workspace context, not packages — but must be walked
    manifest_filenames.insert("settings.gradle");
    manifest_filenames.insert("settings.gradle.kts");
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
fn upsert_package(conn: &Connection, pkg: &PackageInfo) -> Result<String> {
    // Use ON CONFLICT ... DO UPDATE instead of INSERT OR REPLACE to avoid
    // implicit DELETE that triggers FK violations on child tables (dependencies,
    // symbols) which reference packages(name).
    // Also handle path conflicts: if two manifest parsers produce different
    // package names for the same directory, delete the old row first to avoid
    // a UNIQUE constraint violation on packages.path.
    conn.execute(
        "DELETE FROM symbols WHERE package IN (SELECT name FROM packages WHERE path = ?1 AND name != ?2)",
        [&pkg.path, &pkg.name],
    )?;
    conn.execute(
        "DELETE FROM dependencies WHERE package IN (SELECT name FROM packages WHERE path = ?1 AND name != ?2)",
        [&pkg.path, &pkg.name],
    )?;
    conn.execute(
        "DELETE FROM packages WHERE path = ?1 AND name != ?2",
        [&pkg.path, &pkg.name],
    )?;
    conn.execute(
        "INSERT INTO packages (name, path, kind, version, description, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(name) DO UPDATE SET
            path = excluded.path,
            kind = excluded.kind,
            version = excluded.version,
            description = excluded.description,
            metadata = excluded.metadata",
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
    Ok(pkg.name.clone())
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

/// Post-build safety net: clean up any orphaned child rows that reference
/// non-existent packages. This handles edge cases that slip through the
/// per-phase FK management.
fn validate_referential_integrity(conn: &Connection) -> Result<()> {
    let orphaned_syms: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbols WHERE package NOT IN (SELECT name FROM packages)",
        [],
        |row| row.get(0),
    )?;
    let orphaned_deps: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dependencies WHERE package NOT IN (SELECT name FROM packages)",
        [],
        |row| row.get(0),
    )?;

    if orphaned_syms > 0 || orphaned_deps > 0 {
        eprintln!(
            "Warning: cleaning up {} orphaned symbol(s) and {} orphaned dependency(ies)",
            orphaned_syms, orphaned_deps
        );
        conn.execute(
            "DELETE FROM symbols WHERE package NOT IN (SELECT name FROM packages)",
            [],
        )?;
        conn.execute(
            "DELETE FROM dependencies WHERE package NOT IN (SELECT name FROM packages)",
            [],
        )?;
        conn.execute(
            "UPDATE files SET package = NULL WHERE package IS NOT NULL AND package NOT IN (SELECT name FROM packages)",
            [],
        )?;
    }
    Ok(())
}

/// Clear and re-insert symbols for a package using batched multi-row INSERTs.
fn upsert_symbols(conn: &Connection, package: &str, syms: &[symbols::SymbolInfo]) -> Result<()> {
    conn.execute("DELETE FROM symbols WHERE package = ?1", [package])?;

    const BATCH_SIZE: usize = 100;
    const COLS: usize = 10;

    for chunk in syms.chunks(BATCH_SIZE) {
        let placeholders: Vec<String> = (0..chunk.len())
            .map(|i| {
                let base = i * COLS + 1;
                format!(
                    "(?{}, ?{}, ?{}, ?{}, ?{}, ?{}, ?{}, ?{}, ?{}, ?{})",
                    base, base + 1, base + 2, base + 3, base + 4,
                    base + 5, base + 6, base + 7, base + 8, base + 9
                )
            })
            .collect();

        let sql = format!(
            "INSERT INTO symbols (package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, parameters) VALUES {}",
            placeholders.join(", ")
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(chunk.len() * COLS);
        for sym in chunk {
            let params_json = sym
                .parameters
                .as_ref()
                .map(|p| serde_json::to_string(p).unwrap_or_default());

            params.push(Box::new(package.to_string()));
            params.push(Box::new(sym.name.clone()));
            params.push(Box::new(sym.kind.as_str().to_string()));
            params.push(Box::new(sym.signature.clone()));
            params.push(Box::new(sym.file_path.clone()));
            params.push(Box::new(sym.line as i64));
            params.push(Box::new(sym.visibility.clone()));
            params.push(Box::new(sym.parent_symbol.clone()));
            params.push(Box::new(sym.return_type.clone()));
            params.push(Box::new(params_json));
        }

        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    }

    Ok(())
}

/// Batch-upsert source hashes for multiple packages using multi-row INSERT OR REPLACE.
/// Each entry is (package, content_hash). All rows share the same hashed_at timestamp.
fn batch_upsert_source_hashes(conn: &Connection, entries: &[(&str, &str)]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    const BATCH_SIZE: usize = 500;
    const COLS: usize = 3;

    for chunk in entries.chunks(BATCH_SIZE) {
        let placeholders: Vec<String> = (0..chunk.len())
            .map(|i| {
                let base = i * COLS + 1;
                format!("(?{}, ?{}, ?{})", base, base + 1, base + 2)
            })
            .collect();

        let sql = format!(
            "INSERT OR REPLACE INTO source_hashes (package, content_hash, hashed_at) VALUES {}",
            placeholders.join(", ")
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            Vec::with_capacity(chunk.len() * COLS);
        for (package, hash) in chunk {
            params.push(Box::new(package.to_string()));
            params.push(Box::new(hash.to_string()));
            params.push(Box::new(now.clone()));
        }

        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    }

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

/// Clear and re-insert all files using batched multi-row INSERTs.
fn upsert_files(
    conn: &Connection,
    files: &[(String, Option<String>, String, u64)],
) -> Result<()> {
    conn.execute("DELETE FROM files", [])?;

    const BATCH_SIZE: usize = 500;
    const COLS: usize = 4;

    for chunk in files.chunks(BATCH_SIZE) {
        let placeholders: Vec<String> = (0..chunk.len())
            .map(|i| {
                let base = i * COLS + 1;
                format!("(?{}, ?{}, ?{}, ?{})", base, base + 1, base + 2, base + 3)
            })
            .collect();

        let sql = format!(
            "INSERT INTO files (path, package, extension, size_bytes) VALUES {}",
            placeholders.join(", ")
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(chunk.len() * COLS);
        for (path, package, ext, size) in chunk {
            params.push(Box::new(path.clone()));
            params.push(Box::new(package.clone()));
            params.push(Box::new(ext.clone()));
            params.push(Box::new(*size as i64));
        }

        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
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

/// Scan walked settings.gradle files and collect the set of workspace member directories.
fn collect_gradle_settings_context(
    walked: &[WalkedManifest],
) -> (HashSet<String>, HashMap<String, Option<String>>) {
    let mut dirs = HashSet::new();
    let mut root_names: HashMap<String, Option<String>> = HashMap::new(); // settings dir → rootProject.name

    for manifest in walked {
        let filename = manifest
            .abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        if filename != "settings.gradle" && filename != "settings.gradle.kts" {
            continue;
        }

        if let Ok(settings) = gradle_settings::parse_settings_gradle(&manifest.abs_path) {
            for d in &settings.include_dirs {
                let full_dir = if manifest.relative_dir.is_empty() {
                    d.clone()
                } else {
                    format!("{}/{}", manifest.relative_dir, d)
                };
                dirs.insert(full_dir);
            }
            root_names.insert(
                manifest.relative_dir.clone(),
                settings.root_project_name,
            );
        }
    }

    (dirs, root_names)
}

/// Workspace context collected in Phase 1.5 for use during manifest parsing.
struct WorkspaceContext {
    cargo_deps: HashMap<String, String>,
    go_dirs: HashSet<String>,
    maven_parents: HashMap<String, maven::MavenParentContext>,
    gradle_settings: (HashSet<String>, HashMap<String, Option<String>>),
}

/// Summary of a completed build, used for output and metadata storage.
struct BuildSummary {
    num_added: usize,
    num_changed: usize,
    num_removed: usize,
    num_skipped: usize,
    num_source_reextracted: usize,
    num_files: usize,
    total_packages: i64,
    total_symbols: i64,
    failures: Vec<(String, String)>,
}

/// Phase 3: Parse new and changed manifests into packages.
fn phase_parse(
    to_parse: &[&WalkedManifest],
    conn: &Connection,
    parsers: &[Box<dyn ManifestParser>],
    ws: &WorkspaceContext,
) -> Result<(Vec<(String, String, String)>, Vec<(String, String)>)> {
    let mut parsed_packages: Vec<(String, String, String)> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    let cargo_parser = cargo::CargoParser;

    for manifest in to_parse {
        let filename = manifest
            .abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        // Skip context-only files — they provide workspace context, not packages
        if filename == "go.work"
            || filename == "settings.gradle"
            || filename == "settings.gradle.kts"
        {
            continue;
        }

        // Maven: use parent-context-aware parsing
        if filename == "pom.xml" {
            let maven_parser = maven::MavenParser;
            match maven_parser.parse_with_parent_context(
                &manifest.abs_path,
                &manifest.relative_dir,
                &ws.maven_parents,
            ) {
                Ok(pkg) => {
                    let winner = upsert_package(conn, &pkg)?;
                    parsed_packages.push((
                        winner,
                        pkg.path.clone(),
                        pkg.kind.to_string(),
                    ));
                }
                Err(e) => {
                    failures.push((manifest.abs_path.display().to_string(), e.to_string()));
                }
            }
            continue;
        }

        // Gradle: use settings-context-aware parsing
        if filename == "build.gradle" || filename == "build.gradle.kts" {
            let (ref gradle_dirs, ref gradle_root_names) = ws.gradle_settings;
            let settings_ctx = gradle_root_names
                .get(&manifest.relative_dir)
                .map(|name| gradle::GradleSettingsContext {
                    root_project_name: name.clone(),
                });
            match gradle::parse_with_settings_context(
                &manifest.abs_path,
                &manifest.relative_dir,
                &settings_ctx,
            ) {
                Ok(mut pkg) => {
                    if gradle_dirs.contains(&manifest.relative_dir) {
                        pkg.metadata = Some(serde_json::json!({"gradle_workspace": true}));
                    }
                    let winner = upsert_package(conn, &pkg)?;
                    parsed_packages.push((
                        winner,
                        pkg.path.clone(),
                        pkg.kind.to_string(),
                    ));
                }
                Err(e) => {
                    failures.push((manifest.abs_path.display().to_string(), e.to_string()));
                }
            }
            continue;
        }

        // Cargo members: use workspace-aware parsing when context exists
        if filename == "Cargo.toml" && !ws.cargo_deps.is_empty() {
            match cargo_parser.parse_with_workspace_deps(
                &manifest.abs_path,
                &manifest.relative_dir,
                &ws.cargo_deps,
            ) {
                Ok(pkg) => {
                    let winner = upsert_package(conn, &pkg)?;
                    parsed_packages.push((winner, pkg.path.clone(), pkg.kind.to_string()));
                }
                Err(e) => {
                    failures.push((manifest.abs_path.display().to_string(), e.to_string()));
                }
            }
            continue;
        }

        for parser in parsers {
            if parser.filename() == filename {
                match parser.parse(&manifest.abs_path, &manifest.relative_dir) {
                    Ok(mut pkg) => {
                        if pkg.kind == "go" && ws.go_dirs.contains(&manifest.relative_dir) {
                            pkg.metadata = Some(serde_json::json!({"go_workspace": true}));
                        }
                        let winner = upsert_package(conn, &pkg)?;
                        parsed_packages.push((winner, pkg.path.clone(), pkg.kind.to_string()));
                    }
                    Err(e) => {
                        failures.push((manifest.abs_path.display().to_string(), e.to_string()));
                    }
                }
                break;
            }
        }
    }

    // Dedup by path — keep only the last (winning) entry per path.
    // This handles cases where two manifest parsers produce different
    // package names for the same directory.
    {
        let mut by_path: HashMap<String, (String, String, String)> = HashMap::new();
        for entry in parsed_packages.drain(..) {
            by_path.insert(entry.1.clone(), entry);
        }
        parsed_packages = by_path.into_values().collect();
    }

    Ok((parsed_packages, failures))
}

/// Phase 4: Remove packages whose manifests were deleted.
fn phase_remove_deleted(conn: &Connection, removed: &[String]) -> Result<()> {
    for manifest_key in removed {
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
    Ok(())
}

/// Phase 6: Store manifest hashes for parsed manifests using batched multi-row INSERTs.
fn phase_store_hashes(conn: &Connection, to_parse: &[&WalkedManifest]) -> Result<()> {
    const BATCH_SIZE: usize = 500;
    const COLS: usize = 2;

    for chunk in to_parse.chunks(BATCH_SIZE) {
        let placeholders: Vec<String> = (0..chunk.len())
            .map(|i| {
                let base = i * COLS + 1;
                format!("(?{}, ?{})", base, base + 1)
            })
            .collect();

        let sql = format!(
            "INSERT OR REPLACE INTO manifest_hashes (path, content_hash) VALUES {}",
            placeholders.join(", ")
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(chunk.len() * COLS);
        for manifest in chunk {
            params.push(Box::new(manifest.manifest_key.clone()));
            params.push(Box::new(manifest.content_hash.clone()));
        }

        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    }

    Ok(())
}

/// Phase 7: Extract symbols for new/changed packages (parallel).
fn phase_extract_symbols(
    conn: &Connection,
    repo_root: &Path,
    parsed_packages: &[(String, String, String)],
) -> Result<()> {
    let results: Vec<_> = parsed_packages
        .par_iter()
        .map(|(pkg_name, pkg_path, pkg_kind)| {
            let syms = symbols::extract_symbols_for_package(repo_root, pkg_path, pkg_kind);
            let src_hash = hash::compute_source_hash(repo_root, pkg_path, pkg_kind);
            (pkg_name, syms, src_hash)
        })
        .collect();

    let mut hash_entries: Vec<(&str, String)> = Vec::new();
    for (pkg_name, syms, src_hash) in &results {
        match syms {
            Ok(syms) => {
                upsert_symbols(conn, pkg_name, syms)?;
            }
            Err(e) => {
                eprintln!("Warning: symbol extraction failed for {}: {}", pkg_name, e);
            }
        }
        if let Ok(h) = src_hash {
            hash_entries.push((pkg_name.as_str(), h.clone()));
        }
    }

    // Batch-upsert all source hashes collected in this phase
    let refs: Vec<(&str, &str)> = hash_entries.iter().map(|(p, h)| (*p, h.as_str())).collect();
    batch_upsert_source_hashes(conn, &refs)?;
    Ok(())
}

/// Parse an ISO 8601 / RFC 3339 timestamp string into a SystemTime.
fn parse_hashed_at(s: &str) -> Option<std::time::SystemTime> {
    let dt = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    Some(std::time::SystemTime::from(dt))
}

/// Result of parallel phase 8 work for a single package.
enum SourceCheckResult<'a> {
    /// Hash was computed and differs from stored — needs re-extraction.
    Changed(&'a str, Result<Vec<symbols::SymbolInfo>>, String),
    /// Hash was computed but matches stored — just update hashed_at.
    Unchanged(&'a str, String),
}

/// Phase 8: Re-extract symbols for unchanged packages whose source files changed (parallel).
/// Uses mtime pre-check to skip hash computation when no files have been modified.
fn phase_source_incremental(
    conn: &Connection,
    repo_root: &Path,
    unchanged: &[&WalkedManifest],
) -> Result<usize> {
    // Pre-fetch package info, stored hashes, and hashed_at from DB
    let unchanged_pkgs: Vec<(String, String, String, Option<String>, Option<String>)> = unchanged
        .iter()
        .filter_map(|manifest| {
            let relative_dir = &manifest.relative_dir;
            let (pkg_name, pkg_kind): (String, String) = conn
                .query_row(
                    "SELECT name, kind FROM packages WHERE path = ?1",
                    [relative_dir.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok()?;
            let (stored_hash, hashed_at): (Option<String>, Option<String>) = conn
                .query_row(
                    "SELECT content_hash, hashed_at FROM source_hashes WHERE package = ?1",
                    [&pkg_name],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap_or((None, None));
            Some((pkg_name, relative_dir.clone(), pkg_kind, stored_hash, hashed_at))
        })
        .collect();

    // Parallel: mtime pre-check, then conditionally compute hashes and extract symbols
    let results: Vec<SourceCheckResult> = unchanged_pkgs
        .par_iter()
        .filter_map(|(pkg_name, pkg_path, pkg_kind, stored_hash, hashed_at)| {
            // Mtime pre-check: if hashed_at exists and no files are newer, skip entirely
            if let Some(ts_str) = hashed_at {
                if let Some(since) = parse_hashed_at(ts_str) {
                    if !hash::has_newer_source_files(repo_root, pkg_path, pkg_kind, since) {
                        return None; // No files changed — skip hash computation
                    }
                }
            }

            // Mtime says check needed (or no hashed_at) — compute full hash
            let current_hash = hash::compute_source_hash(repo_root, pkg_path, pkg_kind).ok()?;
            if stored_hash.as_deref() == Some(current_hash.as_str()) {
                // Content unchanged — update hashed_at only
                return Some(SourceCheckResult::Unchanged(pkg_name.as_str(), current_hash));
            }
            let syms = symbols::extract_symbols_for_package(repo_root, pkg_path, pkg_kind);
            Some(SourceCheckResult::Changed(pkg_name.as_str(), syms, current_hash))
        })
        .collect();

    // Sequential DB writes
    let mut num_reextracted: usize = 0;
    let mut hash_entries: Vec<(&str, &str)> = Vec::new();
    for result in &results {
        match result {
            SourceCheckResult::Changed(pkg_name, syms, current_hash) => {
                match syms {
                    Ok(syms) => {
                        upsert_symbols(conn, pkg_name, syms)?;
                        num_reextracted += 1;
                    }
                    Err(e) => {
                        eprintln!("Warning: symbol re-extraction failed for {}: {}", pkg_name, e);
                    }
                }
                hash_entries.push((pkg_name, current_hash.as_str()));
            }
            SourceCheckResult::Unchanged(pkg_name, current_hash) => {
                // Update hashed_at to reflect the new computation time
                hash_entries.push((pkg_name, current_hash.as_str()));
            }
        }
    }

    // Batch-upsert all source hashes collected in this phase
    batch_upsert_source_hashes(conn, &hash_entries)?;

    Ok(num_reextracted)
}

/// Phase 9: Walk all files, associate with packages, and insert into DB.
/// Uses a file-tree hash to skip the full rebuild when no files have changed.
fn phase_index_files(
    conn: &Connection,
    repo_root: &Path,
    config: &Config,
) -> Result<usize> {
    let walked_files = walk_files(repo_root, config)?;

    // Compute file-tree hash from (path, size) tuples
    let file_tuples: Vec<(String, u64)> = walked_files
        .iter()
        .map(|f| (f.relative_path.clone(), f.size_bytes))
        .collect();
    let current_hash = hash::compute_file_tree_hash(&file_tuples);

    // Check stored hash
    let stored_hash: Option<String> = conn
        .query_row(
            "SELECT value FROM shire_meta WHERE key = 'file_tree_hash'",
            [],
            |row| row.get(0),
        )
        .ok();

    if stored_hash.as_deref() == Some(current_hash.as_str()) {
        // File tree unchanged — skip rebuild, read count from existing table
        let num_files: usize = conn.query_row(
            "SELECT COUNT(*) FROM files",
            [],
            |row| row.get::<_, i64>(0),
        )? as usize;
        return Ok(num_files);
    }

    // File tree changed (or first build) — full rebuild
    let all_packages: Vec<(String, String)> = conn
        .prepare("SELECT name, path FROM packages")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let associated_files = associate_files_with_packages(&walked_files, &all_packages);

    // Validate package associations against actual DB state to avoid FK violations
    let known_packages: std::collections::HashSet<String> = conn
        .prepare("SELECT name FROM packages")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<std::collections::HashSet<_>, _>>()?;

    let validated_files: Vec<_> = associated_files
        .into_iter()
        .map(|(path, pkg, ext, size)| {
            let valid_pkg = pkg.filter(|p| known_packages.contains(p));
            (path, valid_pkg, ext, size)
        })
        .collect();

    let num_files = validated_files.len();
    upsert_files(conn, &validated_files)?;

    // Store the new file-tree hash
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('file_tree_hash', ?1)",
        [&current_hash],
    )?;

    Ok(num_files)
}

/// Apply config overrides (custom package descriptions).
fn apply_config_overrides(conn: &Connection, config: &Config) -> Result<()> {
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
    Ok(())
}

/// Store build metadata in shire_meta.
fn store_metadata(conn: &Connection, repo_root: &Path, summary: &BuildSummary) -> Result<()> {
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

    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('indexed_at', ?1)",
        [chrono::Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('package_count', ?1)",
        [summary.total_packages.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('symbol_count', ?1)",
        [summary.total_symbols.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('file_count', ?1)",
        [summary.num_files.to_string()],
    )?;
    if let Some(commit) = git_commit {
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('git_commit', ?1)",
            [commit],
        )?;
    }
    Ok(())
}

/// Print build summary to stdout/stderr.
fn print_summary(summary: &BuildSummary, db_path: &Path, is_full_build: bool, force: bool) {
    if !summary.failures.is_empty() {
        eprintln!("{} manifest(s) failed to parse:", summary.failures.len());
        for (path, err) in &summary.failures {
            eprintln!("  {}: {}", path, err);
        }
    }

    if is_full_build || force {
        println!(
            "Indexed {} packages, {} symbols, {} files into {}",
            summary.total_packages, summary.total_symbols, summary.num_files,
            db_path.display()
        );
    } else if summary.num_source_reextracted > 0 {
        println!(
            "Indexed {} packages ({} added, {} updated, {} removed, {} skipped, {} source-updated), {} symbols, {} files into {}",
            summary.total_packages, summary.num_added, summary.num_changed, summary.num_removed,
            summary.num_skipped, summary.num_source_reextracted, summary.total_symbols, summary.num_files,
            db_path.display()
        );
    } else {
        println!(
            "Indexed {} packages ({} added, {} updated, {} removed, {} skipped), {} symbols, {} files into {}",
            summary.total_packages, summary.num_added, summary.num_changed, summary.num_removed,
            summary.num_skipped, summary.total_symbols, summary.num_files,
            db_path.display()
        );
    }
}

/// Print timing breakdown to stderr.
fn print_timings(timings: &[(&str, Duration)], total: Duration) {
    eprintln!("Build timing:");
    for (label, dur) in timings {
        eprintln!("  {:<20} {}ms", label, dur.as_millis());
    }
    eprintln!("  {:<20} {}ms", "total", total.as_millis());
}

pub fn build_index(repo_root: &Path, config: &Config, force: bool) -> Result<()> {
    let build_start = Instant::now();
    let mut timings: Vec<(&str, Duration)> = Vec::new();

    let db_dir = repo_root.join(".shire");
    let db_path = db_dir.join("index.db");
    let conn = db::open_or_create(&db_path)?;

    if force {
        with_transaction(&conn, || {
            conn.execute("DELETE FROM manifest_hashes", [])?;
            conn.execute("DELETE FROM symbols", [])?;
            conn.execute("DELETE FROM source_hashes", [])?;
            conn.execute("DELETE FROM shire_meta WHERE key = 'file_tree_hash'", [])?;
            Ok(())
        })?;
    }

    // Disable FK enforcement during build — the multi-phase pipeline manages
    // referential integrity manually, and a post-build validation pass cleans
    // up any orphaned rows.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;

    let parsers: Vec<Box<dyn ManifestParser>> = vec![
        Box::new(npm::NpmParser),
        Box::new(go::GoParser),
        Box::new(cargo::CargoParser),
        Box::new(python::PythonParser),
        Box::new(maven::MavenParser),
        Box::new(gradle::GradleParser),
        Box::new(gradle::GradleKtsParser),
    ];

    // Phase 1: Walk manifests
    let t = Instant::now();
    let walked = walk_manifests(repo_root, config, &parsers)?;
    timings.push(("walk", t.elapsed()));

    // Phase 1.5: Workspace context
    let t = Instant::now();
    let ws_ctx = WorkspaceContext {
        cargo_deps: collect_cargo_workspace_context(&walked),
        go_dirs: collect_go_workspace_context(&walked),
        maven_parents: maven::collect_maven_parent_context(&walked),
        gradle_settings: collect_gradle_settings_context(&walked),
    };
    timings.push(("workspace-context", t.elapsed()));

    // Phase 2: Diff against stored hashes
    let t = Instant::now();
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
    timings.push(("diff", t.elapsed()));

    // Phase 3: Parse new + changed manifests (transaction-wrapped)
    let t = Instant::now();
    let (parsed_packages, failures) = with_transaction(&conn, || {
        phase_parse(&to_parse, &conn, &parsers, &ws_ctx)
    })?;
    timings.push(("parse", t.elapsed()));

    // Phase 4: Remove deleted packages (transaction-wrapped)
    let t = Instant::now();
    with_transaction(&conn, || {
        phase_remove_deleted(&conn, &diff.removed)
    })?;
    timings.push(("remove-deleted", t.elapsed()));

    // Phase 5: Recompute is_internal (transaction-wrapped)
    let t = Instant::now();
    with_transaction(&conn, || {
        if num_added > 0 || num_changed > 0 || num_removed > 0 {
            recompute_is_internal(&conn)?;
        }
        Ok(())
    })?;
    timings.push(("recompute-internals", t.elapsed()));

    // Phase 6: Store manifest hashes (transaction-wrapped)
    let t = Instant::now();
    with_transaction(&conn, || {
        phase_store_hashes(&conn, &to_parse)
    })?;
    timings.push(("update-hashes", t.elapsed()));

    // Phase 7+8: Extract symbols + source-level re-extraction (transaction-wrapped)
    let t = Instant::now();
    let num_source_reextracted = with_transaction(&conn, || {
        phase_extract_symbols(&conn, repo_root, &parsed_packages)?;
        phase_source_incremental(&conn, repo_root, &diff.unchanged)
    })?;
    timings.push(("extract-symbols", t.elapsed()));

    // Phase 9: Index files (transaction-wrapped)
    let t = Instant::now();
    let num_files = with_transaction(&conn, || {
        phase_index_files(&conn, repo_root, config)
    })?;
    timings.push(("index-files", t.elapsed()));

    // Post-build: config overrides, metadata, summary (transaction-wrapped)
    with_transaction(&conn, || {
        apply_config_overrides(&conn, config)
    })?;

    let total_packages: i64 = conn.query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))?;
    let total_symbols: i64 = conn.query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;

    let summary = BuildSummary {
        num_added,
        num_changed,
        num_removed,
        num_skipped,
        num_source_reextracted,
        num_files,
        total_packages,
        total_symbols,
        failures,
    };

    let total_duration = build_start.elapsed();

    with_transaction(&conn, || {
        store_metadata(&conn, repo_root, &summary)?;
        // Store total build duration in shire_meta
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('total_duration_ms', ?1)",
            [total_duration.as_millis().to_string()],
        )?;
        Ok(())
    })?;

    // Re-enable FK enforcement and validate integrity
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    with_transaction(&conn, || {
        validate_referential_integrity(&conn)
    })?;

    print_summary(&summary, &db_path, is_full_build, force);
    print_timings(&timings, total_duration);

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

    #[test]
    fn test_parse_hashed_at_valid_rfc3339() {
        let result = parse_hashed_at("2026-02-25T10:00:00.000Z");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_hashed_at_invalid() {
        assert!(parse_hashed_at("not-a-timestamp").is_none());
        assert!(parse_hashed_at("").is_none());
    }

    #[test]
    fn test_mtime_precheck_stores_hashed_at() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        // All packages should have hashed_at set after first build
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM source_hashes WHERE hashed_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM source_hashes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, total);
        assert!(total > 0);
    }

    #[test]
    fn test_mtime_precheck_skips_unchanged() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        // First build -- computes all hashes
        build_index(dir.path(), &config, false).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        // Record hashed_at timestamps after first build
        let hashed_at_1: String = conn
            .query_row(
                "SELECT hashed_at FROM source_hashes WHERE package = 'auth-service'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(conn);

        // Second build -- nothing changed, mtime precheck should skip
        build_index(dir.path(), &config, false).unwrap();

        let conn = db::open_readonly(&db_path).unwrap();
        let hashed_at_2: String = conn
            .query_row(
                "SELECT hashed_at FROM source_hashes WHERE package = 'auth-service'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // hashed_at should NOT be updated when mtime precheck skips
        assert_eq!(hashed_at_1, hashed_at_2);
    }
}
