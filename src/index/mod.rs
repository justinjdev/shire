pub mod cargo;
pub mod go;
pub mod hash;
pub mod manifest;
pub mod npm;
pub mod python;

use crate::config::Config;
use crate::db;
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
    let manifest_filenames: HashSet<&str> = parsers.iter().map(|p| p.filename()).collect();
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

pub fn build_index(repo_root: &Path, config: &Config, force: bool) -> Result<()> {
    let db_dir = repo_root.join(".shire");
    let db_path = db_dir.join("index.db");
    let conn = db::open_or_create(&db_path)?;

    // If --force, clear hashes to trigger full rebuild
    if force {
        conn.execute("DELETE FROM manifest_hashes", [])?;
    }

    let parsers: Vec<Box<dyn ManifestParser>> = vec![
        Box::new(npm::NpmParser),
        Box::new(go::GoParser),
        Box::new(cargo::CargoParser),
        Box::new(python::PythonParser),
    ];

    // Phase 1: Walk — discover all manifests with content hashes
    let walked = walk_manifests(repo_root, config, &parsers)?;

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

    for manifest in &to_parse {
        let filename = manifest
            .abs_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        for parser in &parsers {
            if parser.filename() == filename {
                match parser.parse(&manifest.abs_path, &manifest.relative_dir) {
                    Ok(pkg) => {
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

    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('indexed_at', ?1)",
        [chrono::Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('package_count', ?1)",
        [total_packages.to_string()],
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
            "Indexed {} packages into {}",
            total_packages,
            db_path.display()
        );
    } else {
        println!(
            "Indexed {} packages ({} added, {} updated, {} removed, {} skipped) into {}",
            total_packages, num_added, num_changed, num_removed, num_skipped,
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
}
