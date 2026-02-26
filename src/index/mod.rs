pub mod cargo;
pub mod go;
pub mod manifest;
pub mod npm;
pub mod python;

use crate::config::Config;
use crate::db;
use anyhow::Result;
use ignore::WalkBuilder;
use manifest::ManifestParser;
use std::collections::HashSet;
use std::path::Path;

pub fn build_index(repo_root: &Path, config: &Config) -> Result<()> {
    let db_dir = repo_root.join(".shire");
    let db_path = db_dir.join("index.db");
    let conn = db::open_or_create(&db_path)?;

    // Clear existing data for a clean rebuild
    conn.execute_batch(
        "DELETE FROM dependencies; DELETE FROM packages; DELETE FROM packages_fts;",
    )?;

    let parsers: Vec<Box<dyn ManifestParser>> = vec![
        Box::new(npm::NpmParser),
        Box::new(go::GoParser),
        Box::new(cargo::CargoParser),
        Box::new(python::PythonParser),
    ];

    let manifest_filenames: HashSet<&str> = parsers.iter().map(|p| p.filename()).collect();
    let enabled: HashSet<&str> = config
        .discovery
        .manifests
        .iter()
        .map(|s| s.as_str())
        .collect();

    let exclude_set: HashSet<String> = config.discovery.exclude.iter().cloned().collect();

    let mut packages = Vec::new();

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

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().map_or(false, |ft| ft.is_file()) {
            continue;
        }

        let filename = match entry.file_name().to_str() {
            Some(f) => f,
            None => continue,
        };

        if !manifest_filenames.contains(filename) || !enabled.contains(filename) {
            continue;
        }

        let file_path = entry.path();
        let relative_dir = file_path
            .parent()
            .and_then(|p| p.strip_prefix(repo_root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        for parser in &parsers {
            if parser.filename() == filename {
                match parser.parse(file_path, &relative_dir) {
                    Ok(info) => packages.push(info),
                    Err(e) => {
                        eprintln!("Warning: failed to parse {}: {}", file_path.display(), e);
                    }
                }
                break;
            }
        }
    }

    // Collect all package names for is_internal resolution
    let all_names: HashSet<String> = packages.iter().map(|p| p.name.clone()).collect();

    // Insert packages and dependencies
    let mut pkg_stmt = conn.prepare(
        "INSERT OR REPLACE INTO packages (name, path, kind, version, description, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    let mut dep_stmt = conn.prepare(
        "INSERT OR REPLACE INTO dependencies (package, dependency, dep_kind, version_req, is_internal)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;

    for pkg in &packages {
        pkg_stmt.execute((
            &pkg.name,
            &pkg.path,
            pkg.kind,
            &pkg.version,
            &pkg.description,
            &pkg.metadata.as_ref().map(|m| m.to_string()),
        ))?;

        for dep in &pkg.dependencies {
            let is_internal = all_names.contains(&dep.name);
            dep_stmt.execute((
                &pkg.name,
                &dep.name,
                dep.dep_kind.as_str(),
                &dep.version_req,
                is_internal as i32,
            ))?;
        }
    }

    // Apply config overrides
    for override_pkg in &config.packages {
        if let Some(desc) = &override_pkg.description {
            conn.execute(
                "UPDATE packages SET description = ?1 WHERE name = ?2",
                (desc, &override_pkg.name),
            )?;
        }
    }

    // Store metadata
    let git_commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());

    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('indexed_at', ?1)",
        [chrono::Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('package_count', ?1)",
        [packages.len().to_string()],
    )?;
    if let Some(commit) = git_commit {
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('git_commit', ?1)",
            [commit],
        )?;
    }

    println!("Indexed {} packages into {}", packages.len(), db_path.display());
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
        build_index(dir.path(), &config).unwrap();

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
        build_index(dir.path(), &config).unwrap();

        let db_path = dir.path().join(".shire/index.db");
        let conn = db::open_readonly(&db_path).unwrap();

        let results: Vec<String> = conn
            .prepare("SELECT name FROM packages_fts WHERE packages_fts MATCH ?1")
            .unwrap()
            .query_map(["auth"], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(results.contains(&"auth-service".to_string()));
    }
}
