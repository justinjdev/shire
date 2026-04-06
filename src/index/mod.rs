pub mod cargo;
pub mod custom_discovery;
pub mod go;
pub mod go_work;
pub mod gradle;
pub mod gradle_settings;
pub mod hash;
pub mod manifest;
pub mod maven;
pub mod nix;
pub mod npm;
pub mod perl;
pub mod python;
mod ref_writer;
pub mod ruby;

pub use ref_writer::RefWriter;

use crate::config::Config;
use crate::db;
use crate::symbols;
use anyhow::Result;
use ignore::WalkBuilder;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use manifest::{ManifestParser, PackageInfo};
use rayon::prelude::*;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use crate::symbols::walker::PROTO_GENERATED_SUFFIXES;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
        .threads(rayon::current_num_threads().min(8))
        .filter_entry(move |entry| {
            if let Some(name) = entry.file_name().to_str()
                && entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    return !exclude_set.contains(name);
                }
            true
        })
        .build_parallel();

    // Collect manifest paths first (parallel walk)
    let manifest_paths = std::sync::Mutex::new(Vec::new());

    walker.run(|| {
        Box::new(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return ignore::WalkState::Continue;
            }
            let filename = match entry.file_name().to_str() {
                Some(f) => f,
                None => return ignore::WalkState::Continue,
            };
            if !manifest_filenames.contains(filename) || !enabled.contains(filename) {
                return ignore::WalkState::Continue;
            }
            manifest_paths.lock().unwrap().push(entry.into_path());
            ignore::WalkState::Continue
        })
    });

    // Hash manifests in parallel (file reads + SHA-256)
    let paths = manifest_paths.into_inner().unwrap();
    let manifests: Vec<WalkedManifest> = paths
        .into_par_iter()
        .filter_map(|file_path| {
            let filename = file_path.file_name()?.to_str()?.to_string();
            let relative_dir = file_path
                .parent()
                .and_then(|p| p.strip_prefix(repo_root).ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let manifest_key = if relative_dir.is_empty() {
                filename
            } else {
                format!("{}/{}", relative_dir, filename)
            };
            let content_hash = hash::hash_file(&file_path).ok()?;
            Some(WalkedManifest {
                abs_path: file_path,
                relative_dir,
                manifest_key,
                content_hash,
            })
        })
        .collect();

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
        "DELETE FROM symbol_refs WHERE package IN (SELECT name FROM packages WHERE path = ?1 AND name != ?2)",
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
    let orphaned_refs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbol_refs WHERE package IS NOT NULL AND package NOT IN (SELECT name FROM packages)",
        [],
        |row| row.get(0),
    )?;
    // file_id FK is declared ON DELETE CASCADE, but FK enforcement is off
    // during the build pipeline — so file-row deletions during
    // incremental_upsert_files can leave ref rows pointing at a missing
    // files.id. Sweep them up here.
    let orphaned_ref_file_ids: i64 = conn.query_row(
        "SELECT COUNT(*) FROM symbol_refs WHERE file_id NOT IN (SELECT id FROM files)",
        [],
        |row| row.get(0),
    )?;
    let orphaned_deps: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dependencies WHERE package NOT IN (SELECT name FROM packages)",
        [],
        |row| row.get(0),
    )?;

    if orphaned_syms > 0 || orphaned_refs > 0 || orphaned_ref_file_ids > 0 || orphaned_deps > 0 {
        tracing::warn!(
            orphaned_symbols = orphaned_syms,
            orphaned_references = orphaned_refs,
            orphaned_ref_file_ids = orphaned_ref_file_ids,
            orphaned_dependencies = orphaned_deps,
            "cleaning up orphaned symbol(s), reference(s), and dependency(ies)"
        );
        conn.execute(
            "DELETE FROM symbols WHERE package NOT IN (SELECT name FROM packages)",
            [],
        )?;
        conn.execute(
            "DELETE FROM symbol_refs WHERE package IS NOT NULL AND package NOT IN (SELECT name FROM packages)",
            [],
        )?;
        conn.execute(
            "DELETE FROM symbol_refs WHERE file_id NOT IN (SELECT id FROM files)",
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

/// Batch-insert symbols into the symbols table (no DELETE, no trigger management).
/// Callers are responsible for deleting old rows and managing FTS triggers.
fn batch_insert_symbols(conn: &Connection, package: &str, syms: &[symbols::SymbolInfo]) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO symbols (package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, parameters) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;

    for sym in syms {
        let params_json = sym
            .parameters
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_default());

        stmt.execute(rusqlite::params![
            package,
            &sym.name,
            sym.kind.as_str(),
            &sym.signature,
            sym.file_path.as_ref(),
            sym.line as i64,
            sym.visibility.as_str(),
            &sym.parent_symbol,
            &sym.return_type,
            &params_json,
        ])?;
    }

    Ok(())
}

/// Upsert symbols for a package without managing FTS triggers or FTS sync.
/// Caller is responsible for dropping triggers before, rebuilding FTS after.
fn upsert_symbols_no_triggers(conn: &Connection, package: &str, syms: &[symbols::SymbolInfo]) -> Result<()> {
    // Delete old symbols and references (FTS entries will be rebuilt in bulk later)
    conn.execute("DELETE FROM symbols WHERE package = ?1", [package])?;
    conn.execute("DELETE FROM symbol_refs WHERE package = ?1", [package])?;

    // Batch insert new symbols (no triggers fire)
    batch_insert_symbols(conn, package, syms)?;

    Ok(())
}

/// Upsert symbols and references for a single file within a package. Uses triggers (small operation).
fn upsert_symbols_and_refs_for_file(
    conn: &Connection,
    package: &str,
    file_path: &str,
    syms: &[symbols::SymbolInfo],
    refs: &[symbols::ReferenceInfo],
) -> Result<()> {
    // Delete old symbols for this specific file (triggers handle FTS)
    conn.execute(
        "DELETE FROM symbols WHERE package = ?1 AND file_path = ?2",
        rusqlite::params![package, file_path],
    )?;
    // Insert new symbols (triggers handle FTS)
    batch_insert_symbols(conn, package, syms)?;

    // Resolve file_id; if the path is not yet in `files`, we leave the
    // lookup map empty and let `batch_insert_references` synthesize a
    // row (same safety-net path the bulk insert uses). The two walkers
    // can disagree on hidden/symlinked paths — before this unification,
    // the incremental path would early-return with a warn and leave the
    // file without refs, while the bulk path would succeed, so an
    // off→on transition could commit `references_enabled=true` with
    // gaps for walker-missed files.
    let mut file_ids = std::collections::HashMap::new();
    if let Ok(file_id) = conn.query_row(
        "SELECT id FROM files WHERE path = ?1",
        [file_path],
        |row| row.get::<_, i64>(0),
    ) {
        conn.execute(
            "DELETE FROM symbol_refs WHERE file_id = ?1",
            rusqlite::params![file_id],
        )?;
        file_ids.insert(file_path.to_string(), file_id);
    }
    crate::db::queries::batch_insert_references(conn, Some(package), refs, &mut file_ids)?;
    Ok(())
}

/// Batch upsert file hashes for a package.
fn batch_upsert_file_hashes(conn: &Connection, package: &str, file_hashes: &[(&str, &str)]) -> Result<()> {
    if file_hashes.is_empty() {
        return Ok(());
    }

    // Delete old entries for this package
    conn.execute("DELETE FROM file_hashes WHERE package = ?1", [package])?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let mut stmt = conn.prepare_cached(
        "INSERT INTO file_hashes (file_path, package, content_hash, hashed_at) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (fp, hash) in file_hashes {
        stmt.execute(rusqlite::params![fp, package, hash, &now])?;
    }

    Ok(())
}

/// Batch-upsert source hashes for multiple packages.
/// Each entry is (package, content_hash). All rows share the same hashed_at timestamp.
fn batch_upsert_source_hashes(conn: &Connection, entries: &[(&str, &str)]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let mut stmt = conn.prepare_cached(
        "INSERT OR REPLACE INTO source_hashes (package, content_hash, hashed_at) VALUES (?1, ?2, ?3)",
    )?;
    for (package, hash) in entries {
        stmt.execute(rusqlite::params![package, hash, &now])?;
    }

    Ok(())
}

/// A discovered file during file walking.
struct WalkedFile {
    relative_path: String,
    extension: String,
    size_bytes: u64,
}

const MAX_FILES: usize = 500_000;

/// Walk the repo and collect all files with metadata.
fn walk_files(repo_root: &Path, config: &Config) -> Result<Vec<WalkedFile>> {
    let exclude_set: HashSet<String> = config.discovery.exclude.iter().cloned().collect();

    let walker = WalkBuilder::new(repo_root)
        .hidden(true)
        .threads(rayon::current_num_threads().min(8))
        .filter_entry(move |entry| {
            if let Some(name) = entry.file_name().to_str()
                && entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    return !exclude_set.contains(name);
                }
            true
        })
        .build_parallel();

    let files = std::sync::Mutex::new(Vec::new());
    let capped = std::sync::atomic::AtomicBool::new(false);
    let repo_root_ref = repo_root;

    walker.run(|| {
        Box::new(|entry| {
            if capped.load(std::sync::atomic::Ordering::Relaxed) {
                return ignore::WalkState::Quit;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return ignore::WalkState::Continue;
            }

            let file_path = entry.path();
            let relative_path = file_path
                .strip_prefix(repo_root_ref)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            let extension = file_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();

            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);

            let mut guard = files.lock().unwrap();
            guard.push(WalkedFile {
                relative_path,
                extension,
                size_bytes,
            });

            if guard.len() >= MAX_FILES {
                tracing::warn!(max = MAX_FILES, "file tree walk capped at maximum file count");
                capped.store(true, std::sync::atomic::Ordering::Relaxed);
                return ignore::WalkState::Quit;
            }

            ignore::WalkState::Continue
        })
    });

    Ok(files.into_inner().unwrap())
}

/// Associate files with their owning package using longest-prefix matching.
fn associate_files_with_packages(
    files: &[WalkedFile],
    packages: &[(String, String)], // (name, path)
) -> Vec<(String, Option<String>, String, u64)> {
    // Sort package paths by length descending so longest prefix matches first
    let mut sorted_pkgs: Vec<(&str, &str)> = packages.iter().map(|(n, p)| (n.as_str(), p.as_str())).collect();
    sorted_pkgs.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    // Pre-allocate prefix strings with trailing slash to avoid per-file allocations
    let prefixes: Vec<(&str, String)> = sorted_pkgs
        .iter()
        .map(|(name, path)| (*name, format!("{path}/")))
        .collect();

    files
        .iter()
        .map(|file| {
            let file_dir = file
                .relative_path
                .rsplit_once('/')
                .map(|(dir, _)| dir)
                .unwrap_or("");

            let package = prefixes.iter().find_map(|(name, prefix)| {
                if prefix == "/" {
                    // Root-level package matches everything
                    Some((*name).to_string())
                } else if file_dir.starts_with(prefix.as_str()) || file_dir == &prefix[..prefix.len() - 1] {
                    Some((*name).to_string())
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

/// Incrementally update the files table: insert new files, delete removed files,
/// update files whose package/extension/size changed. Avoids a full table wipe.
fn incremental_upsert_files(
    conn: &Connection,
    files: &[(String, Option<String>, String, u64)],
) -> Result<()> {
    // Load existing file paths from DB
    let existing: HashMap<String, (Option<String>, String, i64)> = {
        let mut stmt = conn.prepare("SELECT path, package, extension, size_bytes FROM files")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?),
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, rest) = row?;
            map.insert(path, rest);
        }
        map
    };

    // Build new file set for quick lookup
    let new_set: HashMap<&str, (&Option<String>, &str, u64)> = files
        .iter()
        .map(|(path, pkg, ext, size)| (path.as_str(), (pkg, ext.as_str(), *size)))
        .collect();

    // Delete files no longer present
    let to_delete: Vec<&str> = existing
        .keys()
        .filter(|p| !new_set.contains_key(p.as_str()))
        .map(|p| p.as_str())
        .collect();
    for chunk in to_delete.chunks(500) {
        let placeholders: String = chunk.iter().enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM files WHERE path IN ({})", placeholders);
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = chunk.iter().map(|p| Box::new(p.to_string()) as Box<dyn rusqlite::types::ToSql>).collect();
        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    }

    // Insert new files (not in existing)
    let to_insert: Vec<&(String, Option<String>, String, u64)> = files
        .iter()
        .filter(|(path, _, _, _)| !existing.contains_key(path))
        .collect();

    const BATCH_SIZE: usize = 500;
    const COLS: usize = 4;
    for chunk in to_insert.chunks(BATCH_SIZE) {
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
        for (path, package, ext, size) in chunk.iter() {
            params.push(Box::new(path.clone()));
            params.push(Box::new(package.clone()));
            params.push(Box::new(ext.clone()));
            params.push(Box::new(*size as i64));
        }

        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    }

    // Update existing files whose metadata changed
    let mut update_stmt = conn.prepare(
        "UPDATE files SET package = ?1, extension = ?2, size_bytes = ?3 WHERE path = ?4",
    )?;
    for (path, pkg, ext, size) in files {
        if let Some((old_pkg, old_ext, old_size)) = existing.get(path)
            && (old_pkg != pkg || old_ext != ext || *old_size != *size as i64) {
                update_stmt.execute(rusqlite::params![pkg, ext, *size as i64, path])?;
            }
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

        if filename == "Cargo.toml"
            && let Ok(deps) = cargo::collect_cargo_workspace_deps(&manifest.abs_path) {
                all_ws_deps.extend(deps);
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

        if filename == "go.work"
            && let Ok(use_dirs) = go_work::parse_go_work(&manifest.abs_path) {
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
    num_docs: usize,
    total_packages: i64,
    total_symbols: i64,
    total_references: i64,
    failures: Vec<(String, String)>,
}

/// Phase 3: Parse new and changed manifests into packages.
#[allow(clippy::type_complexity)]
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
            "DELETE FROM file_hashes WHERE package IN (SELECT name FROM packages WHERE path = ?1)",
            [relative_dir],
        )?;
        conn.execute(
            "DELETE FROM symbols WHERE package IN (SELECT name FROM packages WHERE path = ?1)",
            [relative_dir],
        )?;
        conn.execute(
            "DELETE FROM symbol_refs WHERE package IN (SELECT name FROM packages WHERE path = ?1)",
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

/// Result of single-pass hash + extraction for a package.
struct PackageExtractResult {
    pkg_name: String,
    symbols: Vec<symbols::SymbolInfo>,
    references: Vec<symbols::ReferenceInfo>,
    aggregate_hash: String,
    file_hashes: Vec<(String, String)>, // (relative_path, content_hash)
}

/// Intermediate per-file result carrying raw hash bytes for aggregation.
struct FileExtractResult {
    relative_path: String,
    content_hash_hex: String,
    raw_digest: [u8; 32],
    symbols: Vec<symbols::SymbolInfo>,
    references: Vec<symbols::ReferenceInfo>,
}

/// Single-pass: walk source files, read once, hash + extract symbols.
#[allow(clippy::type_complexity)]
fn single_pass_extract(
    repo_root: &Path,
    pkg_path: &str,
    _pkg_kind: &str,
    exclude_extensions: &[String],
    exclude_patterns: &[String],
    skip_references: bool,
) -> Result<(Vec<symbols::SymbolInfo>, Vec<symbols::ReferenceInfo>, String, Vec<(String, String)>)> {
    let package_dir = repo_root.join(pkg_path);
    if !package_dir.is_dir() {
        let empty_hash = hash::hash_bytes_hex(b"");
        return Ok((Vec::new(), Vec::new(), empty_hash, Vec::new()));
    }

    let all_exts = symbols::walker::all_extensions();
    let extensions: Vec<&str> = all_exts
        .into_iter()
        .filter(|ext| {
            let with_dot = format!(".{}", ext);
            !exclude_extensions.contains(&with_dot)
        })
        .collect();
    let source_files = symbols::walker::walk_source_files_with_patterns(&package_dir, &extensions, exclude_patterns)?;

    if source_files.is_empty() {
        let empty_hash = hash::hash_bytes_hex(b"");
        return Ok((Vec::new(), Vec::new(), empty_hash, Vec::new()));
    }

    tracing::debug!(package = %pkg_path, files = source_files.len(), "extracting symbols");

    // Process files in parallel: read once, hash, extract symbols
    let file_results: Vec<FileExtractResult> = source_files
        .par_iter()
        .filter_map(|file_path| {
            let content = std::fs::read(file_path).ok()?;
            let digest = Sha256::digest(&content);
            let raw_digest: [u8; 32] = digest.into();
            let content_hash_hex = format!("{:x}", digest);
            let relative_path = file_path
                .strip_prefix(repo_root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let (syms, refs) = String::from_utf8(content).ok()
                .map(|source| {
                    let file_path_arc: Arc<str> = Arc::from(relative_path.as_str());
                    symbols::extract_file(ext, &source, file_path_arc, skip_references)
                })
                .unwrap_or_else(|| (Vec::new(), Vec::new()));
            Some(FileExtractResult {
                relative_path,
                content_hash_hex,
                raw_digest,
                symbols: syms,
                references: refs,
            })
        })
        .collect();

    // Aggregate: collect symbols and references, build aggregate hash, collect per-file hashes
    let mut all_symbols = Vec::new();
    let mut all_references = Vec::new();
    let mut file_hashes = Vec::new();
    // Sort by path for deterministic aggregate hash
    let mut sorted_results = file_results;
    sorted_results.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let mut hasher = Sha256::new();
    for r in sorted_results {
        all_symbols.extend(r.symbols);
        all_references.extend(r.references);
        // Feed raw digest bytes into aggregate hasher
        hasher.update(r.raw_digest);
        file_hashes.push((r.relative_path, r.content_hash_hex));
    }
    let aggregate_hash = format!("{:x}", hasher.finalize());

    Ok((all_symbols, all_references, aggregate_hash, file_hashes))
}

/// Phase 7: Extract symbols for new/changed packages (parallel).
/// Uses single-pass read+hash+extract to avoid double file reads.
#[allow(clippy::too_many_arguments)]
fn phase_extract_symbols(
    conn: &Connection,
    repo_root: &Path,
    parsed_packages: &[(String, String, String)],
    exclude_extensions: &[String],
    exclude_patterns: &[String],
    progress: &Option<Arc<ProgressBar>>,
    skip_deletes: bool,
    ref_writer: &mut RefWriter,
) -> Result<()> {
    tracing::debug!(packages = parsed_packages.len(), "phase_extract_symbols: extracting symbols for new/changed packages");

    let skip_references = ref_writer.skip_references();
    let results: Vec<_> = parsed_packages
        .par_iter()
        .map(|(pkg_name, pkg_path, pkg_kind)| {
            let result = single_pass_extract(repo_root, pkg_path, pkg_kind, exclude_extensions, exclude_patterns, skip_references);
            if let Some(pb) = progress {
                pb.inc(1);
            }
            match result {
                Ok((symbols, references, aggregate_hash, file_hashes)) => PackageExtractResult {
                    pkg_name: pkg_name.clone(),
                    symbols,
                    references,
                    aggregate_hash,
                    file_hashes,
                },
                Err(e) => {
                    tracing::warn!(package = %pkg_name, error = %e, "symbol extraction failed");
                    PackageExtractResult {
                        pkg_name: pkg_name.clone(),
                        symbols: Vec::new(),
                        references: Vec::new(),
                        aggregate_hash: String::new(),
                        file_hashes: Vec::new(),
                    }
                }
            }
        })
        .collect();

    // Drop FTS triggers once before processing all packages
    db::drop_symbols_fts_triggers(conn)?;
    db::drop_symbol_refs_indexes(conn)?;

    let mut hash_entries: Vec<(&str, String)> = Vec::new();
    for r in &results {
        if skip_deletes {
            // Symbols and refs tables already empty (force/full build) — insert only
            batch_insert_symbols(conn, &r.pkg_name, &r.symbols)?;
            ref_writer.insert(conn, Some(&r.pkg_name), &r.references)?;
        } else {
            upsert_symbols_no_triggers(conn, &r.pkg_name, &r.symbols)?;
            ref_writer.insert(conn, Some(&r.pkg_name), &r.references)?;
        }
        if !r.aggregate_hash.is_empty() {
            hash_entries.push((r.pkg_name.as_str(), r.aggregate_hash.clone()));
        }
        if r.file_hashes.is_empty() {
            conn.execute("DELETE FROM file_hashes WHERE package = ?1", [r.pkg_name.as_str()])?;
        } else {
            let fh_refs: Vec<(&str, &str)> = r.file_hashes.iter().map(|(p, h)| (p.as_str(), h.as_str())).collect();
            batch_upsert_file_hashes(conn, &r.pkg_name, &fh_refs)?;
        }
    }
    // Drop and recreate FTS table, then rebuild from content table.
    // Skip entirely when phase 7 had no packages to process.
    if !results.is_empty() {
        conn.execute_batch("DROP TABLE IF EXISTS symbols_fts")?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
                name, kind, signature, file_path,
                content='symbols',
                content_rowid='rowid',
                tokenize='unicode61 tokenchars ''_-'''
            )",
        )?;
        conn.execute(
            "INSERT INTO symbols_fts(symbols_fts) VALUES('rebuild')",
            [],
        )?;
    }

    // Recreate FTS triggers (needed even if we skipped rebuild)
    db::recreate_symbols_fts_triggers(conn)?;
    db::recreate_symbol_refs_indexes(conn)?;

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

/// Per-file result from incremental check.
struct FileResult {
    file_path: String,        // relative path
    content_hash: String,
    raw_digest: [u8; 32],
    symbols: Option<Vec<symbols::SymbolInfo>>, // None if unchanged
    references: Option<Vec<symbols::ReferenceInfo>>, // None if unchanged
}

/// Result of parallel phase 8 work for a single package.
enum SourceCheckResult {
    /// Package needs per-file updates.
    NeedsUpdate {
        pkg_name: String,
        file_results: Vec<FileResult>,
        deleted_files: Vec<String>, // file paths no longer on disk
        aggregate_hash: String,
    },
    /// Package unchanged — just update hashed_at.
    Unchanged(String, String),
    // Skipped is implicit (filter_map returns None)
}

/// Load stored file hashes for a package from the DB.
fn load_stored_file_hashes(conn: &Connection, package: &str) -> Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT file_path, content_hash FROM file_hashes WHERE package = ?1")?;
    let rows = stmt.query_map([package], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (path, hash) = row?;
        map.insert(path, hash);
    }
    Ok(map)
}

/// Phase 8: Re-extract symbols for unchanged packages whose source files changed (parallel).
/// Uses per-file hashing for granular incremental updates.
fn phase_source_incremental(
    conn: &Connection,
    repo_root: &Path,
    unchanged: &[&WalkedManifest],
    exclude_extensions: &[String],
    exclude_patterns: &[String],
    progress: &Option<Arc<ProgressBar>>,
    ref_writer: &mut RefWriter,
    force_source_reextract: bool,
) -> Result<usize> {
    // Pre-fetch package info, stored hashes, hashed_at, and per-file hashes from DB
    #[allow(clippy::type_complexity)]
    let unchanged_pkgs: Vec<(String, String, String, Option<String>, Option<String>, HashMap<String, String>)> = unchanged
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
            let stored_file_hashes = load_stored_file_hashes(conn, &pkg_name).unwrap_or_default();
            Some((pkg_name, relative_dir.clone(), pkg_kind, stored_hash, hashed_at, stored_file_hashes))
        })
        .collect();

    // Capture skip_references before the parallel section — RefWriter
    // cannot be shared across threads but the bool is Copy.
    let skip_references = ref_writer.skip_references();

    // Parallel: mtime pre-check, then per-file hash comparison and selective extraction
    let results: Vec<SourceCheckResult> = unchanged_pkgs
        .par_iter()
        .filter_map(|(pkg_name, pkg_path, _pkg_kind, _stored_hash, hashed_at, stored_file_hashes)| {
            let result = (|| -> Option<SourceCheckResult> {
                // Mtime pre-check: if hashed_at exists and no files are newer, skip entirely.
                // `force_source_reextract` bypasses this fast-path — used during a
                // references_enabled false→true transition, where refs must populate
                // even for packages whose source files haven't been touched.
                if !force_source_reextract
                    && let Some(ts_str) = hashed_at
                    && let Some(since) = parse_hashed_at(ts_str)
                    && !hash::has_newer_source_files(repo_root, pkg_path, since)
                {
                    return None; // No files changed — skip entirely
                }

                // Mtime says check needed — walk files and do per-file comparison
                let package_dir = repo_root.join(pkg_path);
                if !package_dir.is_dir() {
                    return None;
                }

                let all_exts = symbols::walker::all_extensions();
                let extensions: Vec<&str> = all_exts
                    .into_iter()
                    .filter(|ext| {
                        let with_dot = format!(".{}", ext);
                        !exclude_extensions.contains(&with_dot)
                    })
                    .collect();
                let source_files = symbols::walker::walk_source_files_with_patterns(&package_dir, &extensions, exclude_patterns).ok()?;

                // Process each file: read, hash, compare, extract if changed
                let file_results: Vec<FileResult> = source_files
                    .par_iter()
                    .filter_map(|file_path| {
                        let content = std::fs::read(file_path).ok()?;
                        let digest = Sha256::digest(&content);
                        let raw_digest: [u8; 32] = digest.into();
                        let content_hash = format!("{:x}", digest);
                        let relative_path = file_path
                            .strip_prefix(repo_root)
                            .unwrap_or(file_path)
                            .to_string_lossy()
                            .to_string();

                        let stored = stored_file_hashes.get(&relative_path);
                        if stored == Some(&content_hash) && !force_source_reextract {
                            // File unchanged — include in results for aggregate hash but no symbols.
                            // `force_source_reextract` skips this fast-path so refs
                            // populate for every file during a refs-enabled transition,
                            // while leaving `stored_file_hashes` intact so the caller
                            // can still compute `deleted_files` against it below.
                            Some(FileResult {
                                file_path: relative_path,
                                content_hash,
                                raw_digest,
                                symbols: None,
                                references: None,
                            })
                        } else {
                            // File changed or new — extract symbols and references if valid UTF-8
                            let ext = file_path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("");
                            let (syms, refs) = String::from_utf8(content).ok()
                                .map(|source| {
                                    let file_path_arc: Arc<str> = Arc::from(relative_path.as_str());
                                    symbols::extract_file(ext, &source, file_path_arc, skip_references)
                                })
                                .unwrap_or_else(|| (Vec::new(), Vec::new()));
                            Some(FileResult {
                                file_path: relative_path,
                                content_hash,
                                raw_digest,
                                symbols: Some(syms),
                                references: Some(refs),
                            })
                        }
                    })
                    .collect();

                // Detect deleted files (in stored hashes but not on disk)
                let current_paths: HashSet<&str> = file_results.iter().map(|r| r.file_path.as_str()).collect();
                let deleted_files: Vec<String> = stored_file_hashes
                    .keys()
                    .filter(|p| !current_paths.contains(p.as_str()))
                    .cloned()
                    .collect();

                // Compute aggregate hash from per-file hashes (sorted by path)
                let mut sorted_for_hash: Vec<(&str, &[u8; 32])> = file_results
                    .iter()
                    .map(|r| (r.file_path.as_str(), &r.raw_digest))
                    .collect();
                sorted_for_hash.sort_by(|a, b| a.0.cmp(b.0));

                let mut hasher = Sha256::new();
                for (_, raw_digest) in &sorted_for_hash {
                    hasher.update(*raw_digest);
                }
                let aggregate_hash = format!("{:x}", hasher.finalize());

                // Check if any files actually changed
                let has_changes = file_results.iter().any(|r| r.symbols.is_some()) || !deleted_files.is_empty();
                if !has_changes {
                    return Some(SourceCheckResult::Unchanged(pkg_name.clone(), aggregate_hash));
                }

                Some(SourceCheckResult::NeedsUpdate {
                    pkg_name: pkg_name.clone(),
                    file_results,
                    deleted_files,
                    aggregate_hash,
                })
            })();
            if let Some(pb) = progress {
                pb.inc(1);
            }
            result
        })
        .collect();

    // Sequential DB writes
    let mut num_reextracted: usize = 0;
    let mut hash_entries: Vec<(&str, &str)> = Vec::new();
    for result in &results {
        match result {
            SourceCheckResult::NeedsUpdate { pkg_name, file_results: all_files, deleted_files, aggregate_hash } => {
                // Delete symbols and references for deleted files. symbol_refs
                // keys by file_id, so resolve via the `files` table. Must
                // happen BEFORE `files` rows are removed downstream.
                for del_path in deleted_files {
                    conn.execute(
                        "DELETE FROM symbols WHERE package = ?1 AND file_path = ?2",
                        rusqlite::params![pkg_name, del_path],
                    )?;
                    conn.execute(
                        "DELETE FROM symbol_refs WHERE file_id = (SELECT id FROM files WHERE path = ?1)",
                        rusqlite::params![del_path],
                    )?;
                }
                // Delete file_hashes for deleted files
                for del_path in deleted_files {
                    conn.execute(
                        "DELETE FROM file_hashes WHERE package = ?1 AND file_path = ?2",
                        rusqlite::params![pkg_name, del_path],
                    )?;
                }

                // Collect per-file hashes for batch upsert
                let mut fh_entries: Vec<(&str, &str)> = Vec::new();
                let mut had_changes = false;

                for fr in all_files {
                    fh_entries.push((fr.file_path.as_str(), fr.content_hash.as_str()));
                    if let Some(syms) = &fr.symbols {
                        // File changed — upsert symbols and references for this file
                        let empty_refs = Vec::new();
                        let refs_slice = if ref_writer.skip_references() {
                            &[]
                        } else {
                            fr.references.as_ref().unwrap_or(&empty_refs).as_slice()
                        };
                        upsert_symbols_and_refs_for_file(conn, pkg_name, &fr.file_path, syms, refs_slice)?;
                        had_changes = true;
                    }
                }

                // Update per-file hashes
                batch_upsert_file_hashes(conn, pkg_name, &fh_entries)?;

                if had_changes || !deleted_files.is_empty() {
                    num_reextracted += 1;
                }
                hash_entries.push((pkg_name.as_str(), aggregate_hash.as_str()));
            }
            SourceCheckResult::Unchanged(pkg_name, aggregate_hash) => {
                // Update hashed_at to reflect the new computation time
                hash_entries.push((pkg_name.as_str(), aggregate_hash.as_str()));
            }
        }
    }

    // Batch-upsert all source hashes collected in this phase
    batch_upsert_source_hashes(conn, &hash_entries)?;

    let num_checked = unchanged_pkgs.len();
    let num_skipped_mtime = num_checked - results.len();
    tracing::debug!(
        checked = num_checked,
        skipped_mtime = num_skipped_mtime,
        re_extracted = num_reextracted,
        "phase_source_incremental: incremental source check complete"
    );

    Ok(num_reextracted)
}

/// Backfill `boundary_edges` from the `files` table when the table is empty.
/// Called on fast-path early returns in `phase_index_files` to handle the
/// case where the DB was upgraded (table created empty) but the file tree
/// hasn't changed so the full detection path never runs.
fn backfill_boundary_edges_if_needed(conn: &Connection) -> Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM boundary_edges", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(());
    }
    // Check if there are any proto files at all — skip the query overhead if not
    let has_protos: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE extension = 'proto')",
            [],
            |r| r.get(0),
        )?;
    if !has_protos {
        return Ok(());
    }

    let files: Vec<(String, Option<String>, String, u64)> = conn
        .prepare("SELECT path, package, extension, size_bytes FROM files")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? as u64,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let edges = detect_boundary_edges(conn, &files)?;
    if !edges.is_empty() {
        tracing::debug!(edges = edges.len(), "backfill: boundary edges detected");
        crate::db::queries::batch_insert_boundary_edges(conn, &edges)?;
    }
    Ok(())
}

/// Phase 9: Walk all files, associate with packages, and insert into DB.
/// Uses .git/index mtime as a fast pre-check, then file-tree hash to skip
/// the full rebuild when no files have changed.
fn phase_index_files(
    conn: &Connection,
    repo_root: &Path,
    config: &Config,
) -> Result<usize> {
    // Fast pre-check: if .git/index mtime hasn't changed since last file index,
    // the file tree can't have changed. Skip the expensive walk entirely.
    let git_index_path = repo_root.join(".git/index");
    let stored_file_index_at: Option<String> = conn
        .query_row(
            "SELECT value FROM shire_meta WHERE key = 'file_index_at'",
            [],
            |row| row.get(0),
        )
        .ok();
    if let (Ok(git_meta), Some(stored_ts)) =
        (std::fs::metadata(&git_index_path), &stored_file_index_at)
        && let Ok(git_mtime) = git_meta.modified()
            && let Some(since) = parse_hashed_at(stored_ts) {
                let margin = std::time::Duration::from_secs(1);
                if git_mtime <= since.checked_add(margin).unwrap_or(since) {
                    tracing::debug!("phase_index_files: .git/index unchanged, skipping walk");
                    backfill_boundary_edges_if_needed(conn)?;
                    let num_files: usize = conn
                        .query_row("SELECT COUNT(*) FROM files", [], |row| {
                            row.get::<_, i64>(0)
                        })? as usize;
                    return Ok(num_files);
                }
            }

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
        tracing::debug!("phase_index_files: file tree hash matched, skipping rebuild");
        backfill_boundary_edges_if_needed(conn)?;
        // Update timestamp so mtime pre-check works next time
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('file_index_at', ?1)",
            [&now],
        )?;
        let num_files: usize = conn.query_row(
            "SELECT COUNT(*) FROM files",
            [],
            |row| row.get::<_, i64>(0),
        )? as usize;
        return Ok(num_files);
    }

    tracing::debug!(files = walked_files.len(), "phase_index_files: file tree hash changed, rebuilding file index");

    // File tree changed (or first build) — incremental update
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
    incremental_upsert_files(conn, &validated_files)?;

    // Detect proto→generated boundary edges from the walked file set.
    // Runs after file upsert so package associations are current.
    crate::db::queries::clear_boundary_edges(conn)?;
    let boundary_edges = detect_boundary_edges(conn, &validated_files)?;
    if !boundary_edges.is_empty() {
        tracing::debug!(edges = boundary_edges.len(), "boundary edges detected");
        crate::db::queries::batch_insert_boundary_edges(conn, &boundary_edges)?;
    }

    // Store the new file-tree hash and timestamp for mtime pre-check
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('file_tree_hash', ?1)",
        [&current_hash],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('file_index_at', ?1)",
        [&now],
    )?;

    Ok(num_files)
}

/// Index documentation files: read content from doc files in the files table,
/// extract a title, and upsert into the docs table for FTS search.
fn phase_index_docs(
    conn: &Connection,
    repo_root: &Path,
    config: &Config,
) -> Result<usize> {
    let extensions = &config.docs.extensions;
    if extensions.is_empty() {
        // No doc extensions configured — clear any previously indexed docs
        conn.execute("DELETE FROM docs", [])?;
        return Ok(0);
    }
    let max_size = config.docs.max_file_size;

    // Query files table for doc files by extension
    let placeholders: String = (1..=extensions.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT path, package FROM files WHERE extension IN ({placeholders})"
    );
    let ext_params: Vec<String> = extensions
        .iter()
        .map(|e| e.strip_prefix('.').unwrap_or(e).to_string())
        .collect();
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = ext_params
        .iter()
        .map(|e| Box::new(e.clone()) as Box<dyn rusqlite::types::ToSql>)
        .collect();

    let doc_files: Vec<(String, Option<String>)> = conn
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    if doc_files.is_empty() {
        // No matching doc files found — clear any previously indexed docs
        conn.execute("DELETE FROM docs", [])?;
        return Ok(0);
    }

    // Load existing docs for incremental diff (path → (content_hash, package, size_bytes))
    let existing_docs: HashMap<String, (String, Option<String>, i64)> = {
        let mut stmt = conn.prepare("SELECT path, content_hash, package, size_bytes FROM docs WHERE content_hash IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, (row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, i64>(3)?)))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, state) = row?;
            map.insert(path, state);
        }
        map
    };

    let new_paths: HashSet<&str> = doc_files.iter().map(|(p, _)| p.as_str()).collect();

    // Delete docs no longer in the files table
    let to_delete: Vec<&str> = existing_docs
        .keys()
        .filter(|p| !new_paths.contains(p.as_str()))
        .map(|p| p.as_str())
        .collect();
    for chunk in to_delete.chunks(500) {
        let placeholders: String = chunk.iter().enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM docs WHERE path IN ({})", placeholders);
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = chunk.iter().map(|p| Box::new(p.to_string()) as Box<dyn rusqlite::types::ToSql>).collect();
        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    }

    // Drop FTS triggers for bulk performance (consistent with symbols pattern)
    db::drop_docs_fts_triggers(conn)?;

    // Read and upsert doc files
    let mut upsert_stmt = conn.prepare(
        "INSERT OR REPLACE INTO docs (path, package, title, body, size_bytes, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    let mut count = 0usize;
    let read_limit = max_size as usize + 4; // +4 for max UTF-8 char width
    for (rel_path, package) in &doc_files {
        let abs_path = repo_root.join(rel_path);

        // Read only up to max_file_size + 4 bytes to avoid loading huge files
        let (content, size_bytes) = match read_doc_file(&abs_path, read_limit) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(path = %rel_path, error = %e, "skipping unreadable doc file");
                continue;
            }
        };

        let body = if content.len() > max_size as usize {
            // Find a valid UTF-8 boundary at or before max_size
            let mut end = max_size as usize;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            &content[..end]
        } else {
            content.as_str()
        };

        // Compute hash for incremental check
        let mut hasher = Sha256::new();
        hasher.update(body.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());

        // Skip if content, package, and size are all unchanged
        if let Some((existing_hash, existing_package, existing_size)) = existing_docs.get(rel_path)
            && *existing_hash == content_hash
                && existing_package == package
                && *existing_size == size_bytes
            {
                count += 1;
                continue;
            }

        // Extract title: first markdown heading or first non-empty line
        let title = extract_doc_title(body);

        upsert_stmt.execute(rusqlite::params![rel_path, package, title, body, size_bytes, content_hash])?;
        count += 1;
    }

    // Rebuild FTS index and recreate triggers
    conn.execute("INSERT INTO docs_fts(docs_fts) VALUES('rebuild')", [])?;
    db::recreate_docs_fts_triggers(conn)?;

    Ok(count)
}

/// Extract a title from doc content. For markdown, uses the first `# ` heading.
/// Falls back to the first non-empty line.
fn extract_doc_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Markdown heading
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return Some(heading.trim().to_string());
        }
        // RST title (underlined with = or -)
        // Just return the first non-empty line as title
        return Some(trimmed.to_string());
    }
    None
}

/// Read a doc file, returning (content as UTF-8 string, original file size in bytes).
/// Reads at most `limit` bytes to avoid loading huge files into memory.
fn read_doc_file(path: &Path, limit: usize) -> Result<(String, i64)> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let file_size = file.metadata()?.len() as i64;
    let mut reader = file.take(limit as u64);
    let mut buf = Vec::with_capacity(limit.min(file_size as usize));
    reader.read_to_end(&mut buf)?;
    let content = String::from_utf8(buf)
        .map_err(|_| anyhow::anyhow!("not valid UTF-8"))?;
    Ok((content, file_size))
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
                tracing::warn!(
                    package = %override_pkg.name,
                    "config override matched no packages"
                );
            }
        }
    }
    Ok(())
}

/// Remove stale entries from manifest_hashes and source_hashes that no longer
/// correspond to existing packages. This prevents unbounded growth when packages
/// are renamed, moved, or removed across builds.
fn cleanup_stale_hashes(conn: &Connection) -> Result<()> {
    // source_hashes: key is package name — delete if package no longer exists
    conn.execute(
        "DELETE FROM source_hashes WHERE package NOT IN (SELECT name FROM packages)",
        [],
    )?;

    // file_hashes: key is (file_path, package) — delete if package no longer exists
    conn.execute(
        "DELETE FROM file_hashes WHERE package NOT IN (SELECT name FROM packages)",
        [],
    )?;

    // manifest_hashes: key is manifest path (e.g. "services/auth/package.json").
    // Load known package paths, then delete any manifest hash whose parent dir
    // doesn't match a known package path (or root "").
    let known_paths: HashSet<String> = conn
        .prepare("SELECT path FROM packages")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;

    let all_manifest_keys: Vec<String> = conn
        .prepare("SELECT path FROM manifest_hashes")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    // Workspace-only manifests (go.work, settings.gradle, etc.) don't produce
    // packages but must be kept for workspace context and cached walks.
    const WORKSPACE_MANIFESTS: &[&str] = &["go.work", "settings.gradle", "settings.gradle.kts"];

    let stale_keys: Vec<&str> = all_manifest_keys
        .iter()
        .filter(|key| {
            let filename = key.rsplit_once('/').map(|(_, f)| f).unwrap_or(key.as_str());
            if WORKSPACE_MANIFESTS.contains(&filename) {
                return false; // never prune workspace manifests
            }
            let parent_dir = key.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
            !known_paths.contains(parent_dir)
        })
        .map(|k| k.as_str())
        .collect();

    for chunk in stale_keys.chunks(500) {
        let placeholders: String = chunk
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM manifest_hashes WHERE path IN ({})", placeholders);
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
            .iter()
            .map(|p| Box::new(p.to_string()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
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
                tracing::info!("git rev-parse failed (not a git repo?)");
                None
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not run git");
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
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('reference_count', ?1)",
        [summary.total_references.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('file_count', ?1)",
        [summary.num_files.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('doc_count', ?1)",
        [summary.num_docs.to_string()],
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
            tracing::warn!(path = %path, error = %err, "manifest parse failure");
        }
    }

    if is_full_build || force {
        println!(
            "Indexed {} packages, {} symbols, {} refs, {} files, {} docs into {}",
            summary.total_packages, summary.total_symbols, summary.total_references,
            summary.num_files, summary.num_docs,
            db_path.display()
        );
    } else if summary.num_source_reextracted > 0 {
        println!(
            "Indexed {} packages ({} added, {} updated, {} removed, {} skipped, {} source-updated), {} symbols, {} refs, {} files, {} docs into {}",
            summary.total_packages, summary.num_added, summary.num_changed, summary.num_removed,
            summary.num_skipped, summary.num_source_reextracted, summary.total_symbols,
            summary.total_references, summary.num_files, summary.num_docs,
            db_path.display()
        );
    } else {
        println!(
            "Indexed {} packages ({} added, {} updated, {} removed, {} skipped), {} symbols, {} refs, {} files, {} docs into {}",
            summary.total_packages, summary.num_added, summary.num_changed, summary.num_removed,
            summary.num_skipped, summary.total_symbols, summary.total_references,
            summary.num_files, summary.num_docs,
            db_path.display()
        );
    }
}

/// Check if .git/index has changed since the last build.
/// Returns true if changed or unknown (conservative).
fn git_index_changed_since_build(repo_root: &Path, conn: &Connection) -> bool {
    let git_index_path = repo_root.join(".git/index");
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM shire_meta WHERE key = 'last_build_at'",
            [],
            |row| row.get(0),
        )
        .ok();
    match (std::fs::metadata(&git_index_path), stored) {
        (Ok(meta), Some(ts)) => {
            let git_mtime = match meta.modified() {
                Ok(m) => m,
                Err(_) => return true,
            };
            let since = match parse_hashed_at(&ts) {
                Some(s) => s,
                None => return true,
            };
            let margin = std::time::Duration::from_secs(1);
            git_mtime > since.checked_add(margin).unwrap_or(since)
        }
        _ => true, // unknown — assume changed
    }
}

/// True when `references_enabled` flipped off→on since the last build.
///
/// `prior` is the value read from `shire_meta.references_enabled`:
/// - `Some(true)`: last build already populated `symbol_refs`, nothing to do.
/// - `Some(false)` or `None`: we cannot assume `symbol_refs` is in sync with
///   the current source tree; treat as a transition if refs are now enabled.
///
/// Kept as a pure predicate so the transition logic inside
/// `build_index_inner` is exercised by `cargo test --lib` without having
/// to stand up a full build context.
fn is_refs_transition_enable(current: bool, prior: Option<bool>) -> bool {
    current && prior != Some(true)
}

/// True when a refs transition should force re-hashing. A full build
/// (`is_full_build`) or `--force` run already re-extracts everything, so
/// the hash wipe is redundant in those cases.
fn refs_transition_requires_rehash(
    refs_just_enabled: bool,
    is_full_build: bool,
    force: bool,
) -> bool {
    refs_just_enabled && !is_full_build && !force
}

/// Check if the DB has been populated (has manifest hashes).
fn is_fresh_db(conn: &Connection) -> bool {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM manifest_hashes", [], |row| row.get(0))
        .unwrap_or(0);
    count == 0
}

/// Try to reconstruct manifest walk from cached DB state.
/// Returns None if we can't determine the manifest list (forces full walk).
fn cached_manifest_walk(
    repo_root: &Path,
    conn: &Connection,
) -> Option<Vec<WalkedManifest>> {
    // Read stored manifest paths and hashes from DB
    let mut stmt = conn
        .prepare("SELECT path, content_hash FROM manifest_hashes")
        .ok()?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    if rows.is_empty() {
        return None;
    }

    let mut manifests = Vec::with_capacity(rows.len());
    for (manifest_key, _stored_hash) in &rows {
        let abs_path = repo_root.join(manifest_key);
        if !abs_path.exists() {
            // Manifest was deleted — need full walk to detect removals
            return None;
        }
        // Re-hash to check if content changed
        let current_hash = match hash::hash_file(&abs_path) {
            Ok(h) => h,
            Err(_) => return None,
        };

        let relative_dir = abs_path
            .parent()
            .and_then(|p| p.strip_prefix(repo_root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        manifests.push(WalkedManifest {
            abs_path,
            relative_dir,
            manifest_key: manifest_key.clone(),
            content_hash: current_hash,
        });
    }

    // Check if any NEW manifests have appeared by comparing count with DB
    // This is a heuristic: we can't know for sure without walking, but if
    // the stored count matches and all files exist, it's very likely correct.
    // New manifests will be caught on the next full walk (triggered by force
    // rebuild or when a known manifest changes).
    Some(manifests)
}

/// Print timing breakdown. Emits to stderr when SHIRE_BENCH_TIMINGS is set,
/// otherwise uses tracing::debug.
fn print_timings(timings: &[(&str, Duration)], total: Duration) {
    if std::env::var("SHIRE_BENCH_TIMINGS").is_ok() {
        eprintln!("--- Phase timings ---");
        for (label, dur) in timings {
            eprintln!("  {:25} {:>8.1} ms", label, dur.as_secs_f64() * 1000.0);
        }
        eprintln!("  {:25} {:>8.1} ms", "TOTAL", total.as_secs_f64() * 1000.0);
    }
    tracing::debug!("Build timing:");
    for (label, dur) in timings {
        tracing::debug!(phase = %label, duration_ms = dur.as_millis(), "phase timing");
    }
    tracing::debug!(duration_ms = total.as_millis(), "total build time");
}

/// Create a spinner-style progress bar attached to the MultiProgress.
fn make_spinner(mp: &MultiProgress, msg: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

/// Create a sized progress bar attached to the MultiProgress.
fn make_progress(mp: &MultiProgress, len: u64, msg: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(len));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.cyan} {msg} [{bar:30.cyan/dim}] {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("━╸─"),
    );
    pb.set_message(msg.to_string());
    pb
}

pub fn build_index(repo_root: &Path, config: &Config, force: bool, db_override: Option<&Path>) -> Result<()> {
    build_index_inner(repo_root, config, force, db_override, true)
}

pub fn build_index_quiet(repo_root: &Path, config: &Config, force: bool, db_override: Option<&Path>) -> Result<()> {
    build_index_inner(repo_root, config, force, db_override, false)
}

fn build_index_inner(repo_root: &Path, config: &Config, force: bool, db_override: Option<&Path>, progress: bool) -> Result<()> {
    let build_start = Instant::now();
    let mut timings: Vec<(&str, Duration)> = Vec::new();
    let mp = if progress {
        MultiProgress::new()
    } else {
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
    };

    let wt_info = crate::git::worktree_info(repo_root);
    let db_path = if let Some(p) = db_override {
        p.to_path_buf()
    } else {
        crate::config::resolve_db_path_with_info(config, repo_root, &wt_info)?
    };

    // Seed from main worktree's DB if this is a new linked-worktree build.
    if !db_path.exists()
        && let Some(seed_path) = crate::config::seed_db_path(config, repo_root, &wt_info)?
            && seed_path.exists() {
                crate::db::seed_db(&seed_path, &db_path)?;
                tracing::info!(seed = %seed_path.display(), "seeded DB from main worktree");
                eprintln!("Seeded DB from {}", seed_path.display());
            }

    let conn = db::open_or_create(&db_path, config.rag.enabled)?;

    if force {
        with_transaction(&conn, || {
            conn.execute("DELETE FROM manifest_hashes", [])?;
            conn.execute("DELETE FROM symbols", [])?;
            conn.execute("DELETE FROM symbol_refs", [])?;
            conn.execute("DELETE FROM source_hashes", [])?;
            conn.execute("DELETE FROM file_hashes", [])?;
            conn.execute("DELETE FROM docs", [])?;
            conn.execute("DELETE FROM shire_meta WHERE key = 'file_tree_hash'", [])?;
            // symbol_refs is now empty; mark refs as not-trustworthy
            // atomically with the wipe. If the build fails before
            // phase_extract_symbols re-populates refs, MCP tools will
            // correctly refuse to serve instead of returning silent [].
            crate::db::write_references_enabled(&conn, false)?;
            Ok(())
        })?;
    }

    // Disable FK enforcement during build — the multi-phase pipeline manages
    // referential integrity manually, and a post-build validation pass cleans
    // up any orphaned rows.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;

    // Try MEMORY journal mode to eliminate WAL checkpoint overhead on COMMIT.
    // Falls back silently to WAL if another connection holds a lock.
    let switched_journal: String = conn
        .query_row("PRAGMA journal_mode=MEMORY", [], |row| row.get(0))
        .unwrap_or_else(|_| "wal".to_string());
    let restore_wal = switched_journal == "memory";

    let parsers: Vec<Box<dyn ManifestParser>> = vec![
        Box::new(npm::NpmParser),
        Box::new(go::GoParser),
        Box::new(cargo::CargoParser),
        Box::new(python::PythonParser),
        Box::new(maven::MavenParser),
        Box::new(gradle::GradleParser),
        Box::new(gradle::GradleKtsParser),
        Box::new(perl::CpanfileParser),
        Box::new(ruby::RubyParser),
        Box::new(nix::FlakeNixParser),
    ];

    // Phase 1: Walk manifests
    // On incremental builds, use cached manifest paths to skip the full walk
    // when no manifests have been added or removed.
    tracing::debug!("phase 1: walk manifests");
    let sp = make_spinner(&mp, "Discovering manifests…");
    let t = Instant::now();
    let git_index_changed = force || is_fresh_db(&conn) || git_index_changed_since_build(repo_root, &conn);
    let walked = if !git_index_changed {
        // .git/index unchanged — no files added/removed. Use cached manifest paths.
        match cached_manifest_walk(repo_root, &conn) {
            Some(cached) => {
                tracing::debug!(manifests = cached.len(), "using cached manifest paths");
                cached
            }
            None => walk_manifests(repo_root, config, &parsers)?,
        }
    } else {
        walk_manifests(repo_root, config, &parsers)?
    };
    timings.push(("walk", t.elapsed()));
    sp.finish_with_message(format!("Discovered {} manifests", walked.len()));

    // Phase 1.5: Workspace context
    tracing::debug!("phase 1.5: workspace context");
    let sp = make_spinner(&mp, "Building workspace context…");
    let t = Instant::now();
    let ws_ctx = WorkspaceContext {
        cargo_deps: collect_cargo_workspace_context(&walked),
        go_dirs: collect_go_workspace_context(&walked),
        maven_parents: maven::collect_maven_parent_context(&walked),
        gradle_settings: collect_gradle_settings_context(&walked),
    };
    timings.push(("workspace-context", t.elapsed()));
    sp.finish_with_message("Workspace context ready");

    // Phase 2: Diff against stored hashes
    tracing::debug!("phase 2: diff manifests");
    let sp = make_spinner(&mp, "Diffing manifests…");
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
    sp.finish_with_message(format!(
        "Diff: {} new, {} changed, {} removed, {} unchanged",
        num_added, num_changed, num_removed, num_skipped
    ));

    // Phase 3: Parse new + changed manifests (transaction-wrapped)
    tracing::debug!(to_parse = to_parse.len(), "phase 3: parse manifests");
    let t = Instant::now();
    let pb_parse = if !to_parse.is_empty() {
        let pb = make_progress(&mp, to_parse.len() as u64, "Parsing manifests");
        Some(pb)
    } else {
        None
    };
    let (mut parsed_packages, failures) = with_transaction(&conn, || {
        phase_parse(&to_parse, &conn, &parsers, &ws_ctx)
    })?;
    if let Some(pb) = pb_parse {
        pb.finish_with_message(format!("Parsed {} manifests", to_parse.len()));
    }
    timings.push(("parse", t.elapsed()));

    // Phase 3.5: Custom package discovery
    tracing::debug!("phase 3.5: custom discovery");
    let sp = make_spinner(&mp, "Custom discovery…");
    let t = Instant::now();
    if !config.discovery.custom.is_empty() {
        let known_paths: HashSet<String> = parsed_packages
            .iter()
            .map(|(_, path, _)| path.clone())
            .collect();
        // Also include unchanged packages from DB
        let db_paths: HashSet<String> = conn
            .prepare("SELECT path FROM packages")?
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        let all_known: HashSet<String> = known_paths.union(&db_paths).cloned().collect();

        let global_excludes: HashSet<String> =
            config.discovery.exclude.iter().cloned().collect();

        let custom_pkgs = custom_discovery::discover_custom_packages(
            repo_root,
            &config.discovery.custom,
            &global_excludes,
            &all_known,
        )?;

        if !custom_pkgs.is_empty() {
            with_transaction(&conn, || {
                for pkg in &custom_pkgs {
                    let winner = upsert_package(&conn, pkg)?;
                    parsed_packages.push((winner, pkg.path.clone(), pkg.kind.to_string()));
                }
                Ok(())
            })?;
        }
    }
    timings.push(("custom-discovery", t.elapsed()));
    sp.finish_with_message("Custom discovery done");

    // Phase 4: Remove deleted packages (transaction-wrapped)
    tracing::debug!(removed = diff.removed.len(), "phase 4: remove deleted packages");
    let t = Instant::now();
    if !diff.removed.is_empty() {
        let pb = make_progress(&mp, diff.removed.len() as u64, "Removing deleted");
        with_transaction(&conn, || {
            phase_remove_deleted(&conn, &diff.removed)
        })?;
        pb.finish_with_message(format!("Removed {} packages", diff.removed.len()));
    } else {
        with_transaction(&conn, || {
            phase_remove_deleted(&conn, &diff.removed)
        })?;
    }
    timings.push(("remove-deleted", t.elapsed()));

    // Phase 5: Recompute is_internal (transaction-wrapped)
    tracing::debug!("phase 5: recompute internals");
    let sp = make_spinner(&mp, "Recomputing internals…");
    let t = Instant::now();
    with_transaction(&conn, || {
        if num_added > 0 || num_changed > 0 || num_removed > 0 {
            recompute_is_internal(&conn)?;
        }
        Ok(())
    })?;
    timings.push(("recompute-internals", t.elapsed()));
    sp.finish_with_message("Internals recomputed");

    // Phase 6: Store manifest hashes (transaction-wrapped)
    tracing::debug!("phase 6: store manifest hashes");
    let t = Instant::now();
    if !to_parse.is_empty() {
        let pb = make_progress(&mp, to_parse.len() as u64, "Storing hashes");
        with_transaction(&conn, || {
            phase_store_hashes(&conn, &to_parse)
        })?;
        pb.finish_with_message(format!("Stored {} hashes", to_parse.len()));
    } else {
        with_transaction(&conn, || {
            phase_store_hashes(&conn, &to_parse)
        })?;
    }
    timings.push(("update-hashes", t.elapsed()));

    // Phase 7: Index files (transaction-wrapped).
    //
    // Runs BEFORE symbol+ref extraction so that `files.id` is already assigned
    // for every source path when we insert rows into `symbol_refs` — those
    // rows use `file_id INTEGER` as a compact surrogate for the full path.
    tracing::debug!("phase 7: index files");
    let sp = make_spinner(&mp, "Indexing files…");
    let t = Instant::now();
    let num_files = with_transaction(&conn, || {
        phase_index_files(&conn, repo_root, config)
    })?;
    timings.push(("index-files", t.elapsed()));
    sp.finish_with_message(format!("Indexed {} files", num_files));

    // Phase 8+9: Extract symbols + source-level re-extraction (transaction-wrapped)
    tracing::debug!(
        new_changed = parsed_packages.len(),
        unchanged = diff.unchanged.len(),
        "phase 8+9: extract symbols"
    );
    let t = Instant::now();
    let pb_sym = if !parsed_packages.is_empty() || !diff.unchanged.is_empty() {
        let total = parsed_packages.len() + diff.unchanged.len();
        let pb = make_progress(&mp, total as u64, "Extracting symbols");
        Some(Arc::new(pb))
    } else {
        None
    };
    let pb_sym_clone = pb_sym.clone();
    let refs_enabled = config.symbols.references_enabled;
    // Detect a false→true transition. In the unchanged case, the per-file
    // hash matches the stored hash, so `phase_source_incremental`'s
    // fast-paths skip extraction and leave `symbol_refs` empty for every
    // unchanged file — the user sees a partial ref index with no
    // indication it's stale. We repair by passing `force_source_reextract`
    // into `phase_source_incremental`, which bypasses the mtime and
    // per-file-hash fast-paths while PRESERVING `file_hashes` — the
    // stored hashes are still needed to compute `deleted_files` (files
    // removed since the last build). Wiping the hashes here would break
    // deleted-file cleanup.
    let prior_refs_enabled = crate::db::read_references_enabled(&conn);
    let refs_just_enabled = is_refs_transition_enable(refs_enabled, prior_refs_enabled);
    let force_source_reextract =
        refs_transition_requires_rehash(refs_just_enabled, is_full_build, force);
    if force_source_reextract {
        tracing::warn!(
            prior = ?prior_refs_enabled,
            "references_enabled transitioned to true — forcing source re-extraction \
             so symbol_refs is populated for every source file"
        );
    }
    let num_source_reextracted = with_transaction(&conn, || {
        // Wipe symbol_refs if the user has turned the experimental refs
        // feature off — this keeps the DB from carrying stale refs while
        // disabled. Cheap: DELETE FROM without a WHERE is O(1) in SQLite
        // with the table truncated-and-recreated optimization.
        if !refs_enabled {
            conn.execute("DELETE FROM symbol_refs", [])?;
        }
        let mut ref_writer = RefWriter::new(&conn, refs_enabled)?;
        phase_extract_symbols(&conn, repo_root, &parsed_packages, &config.symbols.exclude_extensions, &config.symbols.exclude_patterns, &pb_sym_clone, is_full_build || force, &mut ref_writer)?;
        let count = if git_index_changed || refs_just_enabled {
            // The refs_just_enabled branch is load-bearing: flipping the
            // flag in shire.toml does not touch .git/index, so without
            // this override phase_source_incremental would skip unchanged
            // packages and leave symbol_refs empty for every file the
            // user hasn't edited since the last build.
            phase_source_incremental(&conn, repo_root, &diff.unchanged, &config.symbols.exclude_extensions, &config.symbols.exclude_patterns, &pb_sym_clone, &mut ref_writer, force_source_reextract)?
        } else {
            // .git/index unchanged — no tracked files can have changed, skip per-file mtime walks
            if let Some(pb) = &pb_sym_clone {
                pb.inc(diff.unchanged.len() as u64);
            }
            0
        };
        // Commit the refs-trustworthy flag atomically with the extraction
        // results. If a later phase (docs, rag, meta) fails, the flag's
        // committed state still matches the committed state of
        // symbol_refs — so MCP tools cannot return silent [] from a
        // partially-repopulated table.
        crate::db::write_references_enabled(&conn, refs_enabled)?;
        Ok(count)
    })?;
    if let Some(pb) = pb_sym {
        pb.finish_with_message("Symbols extracted");
    }
    timings.push(("extract-symbols", t.elapsed()));

    // Phase 9.5: Index documentation content
    tracing::debug!("phase 9.5: index docs");
    let sp = make_spinner(&mp, "Indexing docs…");
    let t = Instant::now();
    let num_docs = with_transaction(&conn, || {
        phase_index_docs(&conn, repo_root, config)
    })?;
    timings.push(("index-docs", t.elapsed()));
    sp.finish_with_message(format!("Indexed {} docs", num_docs));

    // Phase 10: RAG file-level embedding (optional, runs in background thread)
    // Build completes and prints summary immediately. Embedding continues in a
    // background thread that opens its own DB connection. The thread handle is
    // returned so callers can optionally wait for it.
    #[cfg(feature = "rag")]
    let rag_handle: Option<std::thread::JoinHandle<()>> = if config.rag.enabled {
        use crate::rag::embedder::{embed_files, Embedder, FileForEmbedding, FileSymbol};

        let changed_packages: Vec<String> = parsed_packages
            .iter()
            .map(|(name, _, _)| name.clone())
            .collect();

        if changed_packages.is_empty() {
            tracing::debug!("rag: no changed packages, skipping embedding");
            None
        } else {
            // Clean stale embeddings (fast, inline)
            conn.execute(
                "DELETE FROM file_embeddings WHERE file_id NOT IN (SELECT id FROM files)",
                [],
            )?;

            // Collect files needing embeddings (fast DB reads)
            let placeholders: String = (1..=changed_packages.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let params: Vec<Box<dyn rusqlite::types::ToSql>> = changed_packages
                .iter()
                .map(|p| Box::new(p.clone()) as Box<dyn rusqlite::types::ToSql>)
                .collect();
            // Delete existing embeddings for changed packages so we can re-embed
            conn.execute(
                &format!(
                    "DELETE FROM file_embeddings WHERE file_id IN \
                     (SELECT id FROM files WHERE package IN ({placeholders}))"
                ),
                rusqlite::params_from_iter(params.iter()),
            )?;

            let file_sql = format!(
                "SELECT f.id, f.path, f.package \
                 FROM files f \
                 WHERE f.package IN ({placeholders}) \
                 AND EXISTS (SELECT 1 FROM symbols s WHERE s.file_path = f.path)"
            );
            let file_inputs: Vec<FileForEmbedding> = conn
                .prepare(&file_sql)?
                .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?.unwrap_or_default()))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|(file_id, file_path, package)| {
                    let symbols: Vec<FileSymbol> = conn
                        .prepare("SELECT name, kind, signature FROM symbols WHERE file_path = ?1")
                        .and_then(|mut s| {
                            s.query_map([file_path.as_str()], |row| {
                                Ok(FileSymbol {
                                    name: row.get(0)?,
                                    kind: row.get(1)?,
                                    signature: row.get(2)?,
                                })
                            })
                            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                        })
                        .unwrap_or_else(|e| {
                            tracing::debug!(error = %e, file = %file_path, "failed to load symbols for embedding");
                            Vec::new()
                        });
                    FileForEmbedding {
                        id: file_id,
                        file_path,
                        package,
                        symbols,
                    }
                })
                .collect();

            if file_inputs.is_empty() {
                None
            } else {
                let num_files = file_inputs.len();
                let db_path_owned = db_path.clone();
                let rag_config = config.rag.clone();
                let show_progress = progress;

                Some(std::thread::spawn(move || {
                    let pb = ProgressBar::new(num_files as u64);
                    if !show_progress {
                        pb.set_draw_target(ProgressDrawTarget::hidden());
                    }
                    pb.set_style(
                        ProgressStyle::default_bar()
                            .template("{spinner:.cyan} {msg} [{bar:30.cyan/dim}] {pos}/{len} ({eta})")
                            .expect("hardcoded progress template must be valid")
                            .progress_chars("━╸─"),
                    );
                    pb.set_message("Embedding files");

                    let t = Instant::now();
                    let embedder = match Embedder::new(&rag_config) {
                        Ok(e) => e,
                        Err(e) => {
                            pb.finish_with_message(format!("Embedding failed: {e}"));
                            tracing::warn!(error = %e, "RAG background: model init failed");
                            return;
                        }
                    };
                    match embed_files(&embedder, &file_inputs, |n| pb.inc(n as u64)) {
                        Ok(embeddings) => {
                            match db::open_or_create(&db_path_owned, true) {
                                Ok(bg_conn) => {
                                    if let Err(e) = crate::rag::storage::insert_file_embeddings(&bg_conn, &embeddings) {
                                        pb.finish_with_message(format!("Embedding failed: {e}"));
                                        tracing::warn!(error = %e, "RAG background: insert failed");
                                    } else {
                                        pb.finish_with_message(format!(
                                            "Embedded {num_files} files in {:.1}s",
                                            t.elapsed().as_secs_f64()
                                        ));
                                    }
                                }
                                Err(e) => {
                                    pb.finish_with_message(format!("Embedding failed: {e}"));
                                    tracing::warn!(error = %e, "RAG background: DB open failed");
                                }
                            }
                        }
                        Err(e) => {
                            pb.finish_with_message(format!("Embedding failed: {e}"));
                            tracing::warn!(error = %e, "RAG background: embed failed");
                        }
                    }
                }))
            }
        }
    } else {
        None
    };

    #[cfg(not(feature = "rag"))]
    let rag_handle: Option<std::thread::JoinHandle<()>> = None;

    // Post-build: config overrides, metadata, summary (transaction-wrapped)
    with_transaction(&conn, || {
        apply_config_overrides(&conn, config)
    })?;

    let total_packages: i64 = conn.query_row("SELECT COUNT(*) FROM packages", [], |row| row.get(0))?;
    let total_symbols: i64 = conn.query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
    let total_references: i64 = conn.query_row("SELECT COUNT(*) FROM symbol_refs", [], |row| row.get(0))?;

    let summary = BuildSummary {
        num_added,
        num_changed,
        num_removed,
        num_skipped,
        num_source_reextracted,
        num_files,
        num_docs,
        total_packages,
        total_symbols,
        total_references,
        failures,
    };

    let total_duration = build_start.elapsed();

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    with_transaction(&conn, || {
        store_metadata(&conn, repo_root, &summary)?;
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('total_duration_ms', ?1)",
            [total_duration.as_millis().to_string()],
        )?;
        // Timestamp for .git/index mtime fast-path on next build
        conn.execute(
            "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('last_build_at', ?1)",
            [&now],
        )?;
        // NOTE: `references_enabled` is written inside the
        // phase_extract_symbols transaction (see build_index_inner), so
        // the flag commits atomically with the symbol_refs mutation. Not
        // written here to avoid a later-phase failure desynchronizing
        // the flag from the committed state of symbol_refs.
        Ok(())
    })?;

    // Re-enable FK enforcement and validate integrity
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    with_transaction(&conn, || {
        validate_referential_integrity(&conn)
    })?;

    // Clean up stale hash entries that no longer correspond to any existing package.
    // source_hashes keys on package name, so orphans are easy to detect.
    // manifest_hashes keys on manifest path — orphans occur when packages are
    // renamed/moved; we detect them by checking if the manifest's parent directory
    // still matches a known package path.
    cleanup_stale_hashes(&conn)?;

    // FTS5 maintenance: incremental merge on non-full builds.
    // Skip optimize on full/forced builds — the FTS was just rebuilt from scratch.
    if !is_full_build && !force {
        tracing::debug!("FTS maintenance: incremental merge");
        conn.execute_batch(
            "INSERT INTO packages_fts(packages_fts, rank) VALUES('merge', 500);
             INSERT INTO files_fts(files_fts, rank) VALUES('merge', 500);
             INSERT INTO symbols_fts(symbols_fts, rank) VALUES('merge', 500);
             INSERT INTO docs_fts(docs_fts, rank) VALUES('merge', 500);",
        )?;
    }

    // Reclaim free pages from incremental updates (prevents DB bloat over time)
    conn.execute_batch("PRAGMA incremental_vacuum(100);")?;

    // Wait for RAG embedding to complete before restoring journal mode.
    // The background RAG thread opened its own connection to this DB; switching
    // back to WAL requires an exclusive lock, which can fail with SQLITE_BUSY
    // if the RAG thread's connection is still active.
    if let Some(handle) = rag_handle
        && let Err(panic_payload) = handle.join() {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic_payload.downcast_ref::<String>().map(|s| s.as_str()))
                .unwrap_or("unknown panic");
            tracing::error!(panic = %msg, "RAG embedding thread panicked");
        }

    // Restore WAL mode for read-heavy query workloads after the build.
    if restore_wal {
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    }

    print_summary(&summary, &db_path, is_full_build, force);
    print_timings(&timings, total_duration);

    Ok(())
}

/// Detect proto→generated-code boundary edges from walked files.
///
/// Scans walked files for `.proto` files and files matching known generated
/// suffixes. Matches by stem (filename without extension/suffix), then filters
/// by scope: same package, dependent package (via `dependencies` table), or
/// sibling package (shared parent directory).
fn detect_boundary_edges(
    conn: &Connection,
    files: &[(String, Option<String>, String, u64)], // (path, package, extension, size)
) -> Result<Vec<crate::db::queries::BoundaryEdge>> {
    // Collect proto stems: stem → Vec<(path, package)>
    let mut proto_map: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    // Collect generated stems: stem → Vec<(path, package)>
    let mut generated_map: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();

    for (path, package, extension, _size) in files {
        let filename = path.rsplit_once('/').map(|(_, f)| f).unwrap_or(path);

        if extension == "proto" {
            let stem = filename.strip_suffix(".proto").unwrap_or(filename);
            proto_map
                .entry(stem.to_string())
                .or_default()
                .push((path.clone(), package.clone()));
            continue;
        }

        for suffix in PROTO_GENERATED_SUFFIXES {
            if let Some(stem) = filename.strip_suffix(suffix) {
                generated_map
                    .entry(stem.to_string())
                    .or_default()
                    .push((path.clone(), package.clone()));
                break; // a file matches at most one suffix
            }
        }
    }

    if proto_map.is_empty() || generated_map.is_empty() {
        return Ok(Vec::new());
    }

    // Load package name→path mapping for sibling-directory comparison
    let pkg_paths: HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT name, path FROM packages")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    // Load dependency edges for scope filtering: set of (dependent, dependency)
    let dep_edges: HashSet<(String, String)> = {
        let mut stmt =
            conn.prepare("SELECT package, dependency FROM dependencies WHERE is_internal = 1")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut edges = Vec::new();

    for (stem, protos) in &proto_map {
        let gen_files = match generated_map.get(stem) {
            Some(g) => g,
            None => continue,
        };

        for (proto_path, proto_pkg) in protos {
            let proto_pkg_path = proto_pkg
                .as_deref()
                .and_then(|n| pkg_paths.get(n))
                .map(|s| s.as_str());
            let proto_parent = proto_pkg_path.and_then(package_parent);

            for (gen_path, gen_pkg) in gen_files {
                let gen_pkg_path = gen_pkg
                    .as_deref()
                    .and_then(|n| pkg_paths.get(n))
                    .map(|s| s.as_str());
                if !is_in_scope(
                    proto_pkg.as_deref(),
                    gen_pkg.as_deref(),
                    gen_pkg_path,
                    &proto_parent,
                    &dep_edges,
                ) {
                    continue;
                }

                edges.push(crate::db::queries::BoundaryEdge {
                    source_path: proto_path.clone(),
                    generated_path: gen_path.clone(),
                    source_package: proto_pkg.clone(),
                    generated_package: gen_pkg.clone(),
                    kind: "proto".into(),
                });
            }
        }
    }

    Ok(edges)
}

/// Extract the parent directory of a package path for sibling-package matching.
/// "services/auth/proto" → "services/auth", "proto" → None
fn package_parent(pkg_path: &str) -> Option<String> {
    pkg_path.rsplit_once('/').map(|(parent, _)| parent.to_string())
}

/// Check if a generated file is in scope relative to its proto source.
/// Accepts the pair if: same package, generated depends on proto's package,
/// both packages share a parent directory, or either file has no package
/// association (unpackaged files are always accepted).
fn is_in_scope(
    proto_pkg: Option<&str>,
    gen_pkg: Option<&str>,
    gen_pkg_path: Option<&str>,
    proto_parent: &Option<String>,
    dep_edges: &HashSet<(String, String)>,
) -> bool {
    match (proto_pkg, gen_pkg) {
        (Some(pp), Some(gp)) => {
            if pp == gp {
                return true;
            }
            if dep_edges.contains(&(gp.to_string(), pp.to_string())) {
                return true;
            }
            if let Some(proto_par) = proto_parent
                && let Some(gen_par) = gen_pkg_path.and_then(package_parent)
                && *proto_par == gen_par
            {
                return true;
            }
            false
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn test_is_refs_transition_enable_off_to_on() {
        // None/false→true is the load-bearing transition: symbol_refs
        // cannot be trusted without a re-extraction pass.
        assert!(is_refs_transition_enable(true, None));
        assert!(is_refs_transition_enable(true, Some(false)));
    }

    #[test]
    fn test_is_refs_transition_enable_no_change() {
        // Already-on or never-enabled: no transition.
        assert!(!is_refs_transition_enable(true, Some(true)));
        assert!(!is_refs_transition_enable(false, Some(false)));
        assert!(!is_refs_transition_enable(false, None));
    }

    #[test]
    fn test_is_refs_transition_enable_on_to_off() {
        // On→off is handled separately (wipe symbol_refs) — not a refs
        // transition this predicate cares about.
        assert!(!is_refs_transition_enable(false, Some(true)));
    }

    #[test]
    fn test_refs_transition_requires_rehash_default_path() {
        // Incremental build with a genuine transition: must re-hash.
        assert!(refs_transition_requires_rehash(true, false, false));
    }

    #[test]
    fn test_refs_transition_requires_rehash_skips_when_full_build() {
        // A full build already re-extracts everything; wiping hashes on
        // top is redundant work.
        assert!(!refs_transition_requires_rehash(true, true, false));
    }

    #[test]
    fn test_refs_transition_requires_rehash_skips_when_forced() {
        // --force paths already walk every file; same rationale.
        assert!(!refs_transition_requires_rehash(true, false, true));
    }

    #[test]
    fn test_refs_transition_requires_rehash_skips_when_no_transition() {
        // No transition → nothing to repair.
        assert!(!refs_transition_requires_rehash(false, false, false));
        assert!(!refs_transition_requires_rehash(false, true, false));
        assert!(!refs_transition_requires_rehash(false, false, true));
    }

    /// Regression test: `upsert_symbols_and_refs_for_file` must not
    /// early-return when the file_path is missing from the `files`
    /// table. It should synthesize the row the same way the bulk path
    /// (`batch_insert_references`) does, otherwise a walker-missed file
    /// commits with no refs while `references_enabled=true` is also
    /// committed — the exact silent-[] gap the MCP guard exists to
    /// prevent.
    #[test]
    fn test_upsert_symbols_and_refs_synthesizes_files_row_when_missing() {
        use crate::symbols::{ReferenceInfo, ReferenceKind};
        use std::sync::Arc;

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.db");
        let conn = db::open_or_create(&path, false).unwrap();

        // Seed a package row (NOT a files row for the path we're about
        // to process). This simulates the walker-mismatch scenario.
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('svc', 'svc', 'go')",
            [],
        )
        .unwrap();

        let walker_missed_path = ".hidden/config.go";
        let refs = vec![ReferenceInfo {
            name: "Foo".into(),
            kind: ReferenceKind::Call,
            file_path: Arc::from(walker_missed_path),
            line: 10,
            enclosing_symbol: Some("Bar".into()),
        }];

        upsert_symbols_and_refs_for_file(&conn, "svc", walker_missed_path, &[], &refs).unwrap();

        // The ref should be present (walker gap did not cause it to
        // disappear), and the files row should have been synthesized.
        let ref_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbol_refs WHERE name = 'Foo'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ref_count, 1, "ref must be inserted even when file_id is missing");

        let file_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE path = ?1",
                [walker_missed_path],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(file_count, 1, "files row must be synthesized by the bulk path");
    }

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
        build_index(dir.path(), &config, false, None).unwrap();

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
        build_index(dir.path(), &config, false, None).unwrap();

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
        build_index(dir.path(), &config, false, None).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);
        assert_eq!(hash_count(dir.path()), 3);

        // Second build — nothing changed
        build_index(dir.path(), &config, false, None).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);
    }

    #[test]
    fn test_incremental_modified_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        build_index(dir.path(), &config, false, None).unwrap();

        // Modify auth-service version
        let auth_path = dir.path().join("services/auth/package.json");
        fs::write(
            &auth_path,
            br#"{"name": "auth-service", "version": "2.0.0", "description": "Auth v2", "dependencies": {"shared-types": "^1.0"}}"#,
        ).unwrap();

        build_index(dir.path(), &config, false, None).unwrap();

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

        build_index(dir.path(), &config, false, None).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);

        // Delete the Go package
        fs::remove_file(dir.path().join("services/gateway/go.mod")).unwrap();

        build_index(dir.path(), &config, false, None).unwrap();
        assert_eq!(pkg_count(dir.path()), 2);
        assert_eq!(hash_count(dir.path()), 2);
    }

    #[test]
    fn test_incremental_added_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());
        let config = Config::default();

        build_index(dir.path(), &config, false, None).unwrap();
        assert_eq!(pkg_count(dir.path()), 3);

        // Add a new npm package
        let new_dir = dir.path().join("services/billing");
        fs::create_dir_all(&new_dir).unwrap();
        fs::write(
            new_dir.join("package.json"),
            br#"{"name": "billing", "version": "1.0.0", "description": "Billing service"}"#,
        ).unwrap();

        build_index(dir.path(), &config, false, None).unwrap();
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
        build_index(dir.path(), &config, false, None).unwrap();

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

        build_index(dir.path(), &config, false, None).unwrap();

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

        build_index(dir.path(), &config, false, None).unwrap();
        assert_eq!(hash_count(dir.path()), 3);

        // Force rebuild — should still work and produce same result
        build_index(dir.path(), &config, true, None).unwrap();
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
        build_index(root, &config, false, None).unwrap();

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
        build_index(dir.path(), &config, false, None).unwrap();

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
        build_index(dir.path(), &config, false, None).unwrap();

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

        build_index(dir.path(), &config, false, None).unwrap();

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
        build_index(dir.path(), &config, false, None).unwrap();

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
        build_index(dir.path(), &config, false, None).unwrap();

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

    #[test]
    fn test_extract_doc_title_markdown_heading() {
        assert_eq!(
            extract_doc_title("# My Title\n\nSome content"),
            Some("My Title".to_string())
        );
    }

    #[test]
    fn test_extract_doc_title_leading_blank_lines() {
        assert_eq!(
            extract_doc_title("\n\n# Title After Blanks\n"),
            Some("Title After Blanks".to_string())
        );
    }

    #[test]
    fn test_extract_doc_title_no_heading() {
        assert_eq!(
            extract_doc_title("Just plain text\nMore text"),
            Some("Just plain text".to_string())
        );
    }

    #[test]
    fn test_extract_doc_title_empty() {
        assert_eq!(extract_doc_title(""), None);
        assert_eq!(extract_doc_title("\n\n\n"), None);
    }

    #[test]
    fn test_phase_index_docs_with_utf8_truncation() {
        // Verify that multi-byte UTF-8 characters at the truncation boundary
        // don't cause a panic. "é" is 2 bytes, "日" is 3 bytes.
        let dir = tempfile::TempDir::new().unwrap();
        create_test_monorepo(dir.path());

        let config = Config {
            docs: crate::config::DocsConfig {
                extensions: vec![".md".into()],
                max_file_size: 5, // truncate at 5 bytes
            },
            ..Config::default()
        };

        // Build index first to populate files table
        let db_path = dir.path().join(".shire/index.db");
        build_index(dir.path(), &config, true, Some(&db_path)).unwrap();

        // Create a doc file with multi-byte chars near the boundary
        let doc_path = dir.path().join("services/auth/README.md");
        fs::write(&doc_path, "abc日本語").unwrap(); // "abc" = 3 bytes, "日" starts at byte 3 (3 bytes)

        // Rebuild to index the doc file
        build_index(dir.path(), &config, true, Some(&db_path)).unwrap();

        let conn = db::open_readonly(&db_path).unwrap();
        let body: String = conn
            .query_row(
                "SELECT body FROM docs WHERE path LIKE '%README.md'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // max_file_size=5, "abc日" would be 6 bytes, so truncated to "abc" (3 bytes at char boundary)
        assert_eq!(body, "abc");
        assert!(body.len() <= 5);
    }

    #[test]
    fn test_detect_boundary_edges_matches_by_stem_and_scope() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::create_schema_for_test(&conn);

        // Two packages: proto-pkg and go-pkg in the same parent dir
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('proto-pkg', 'services/auth/proto', 'proto')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('go-pkg', 'services/auth/gen', 'go')",
            [],
        )
        .unwrap();
        // billing is in a different parent — should NOT match
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('billing-pkg', 'services/billing/gen', 'go')",
            [],
        )
        .unwrap();

        let files: Vec<(String, Option<String>, String, u64)> = vec![
            (
                "services/auth/proto/user.proto".into(),
                Some("proto-pkg".into()),
                "proto".into(),
                100,
            ),
            (
                "services/auth/gen/user.pb.go".into(),
                Some("go-pkg".into()),
                "go".into(),
                200,
            ),
            (
                "services/auth/gen/user_pb2.py".into(),
                Some("go-pkg".into()),
                "py".into(),
                150,
            ),
            // Out-of-scope: different parent directory, no dependency
            (
                "services/billing/gen/user.pb.go".into(),
                Some("billing-pkg".into()),
                "go".into(),
                200,
            ),
        ];

        let edges = detect_boundary_edges(&conn, &files).unwrap();

        assert_eq!(
            edges.len(),
            2,
            "should match user.pb.go and user_pb2.py in sibling package"
        );
        assert!(edges
            .iter()
            .all(|e| e.source_path == "services/auth/proto/user.proto"));
        let gen_paths: std::collections::HashSet<&str> =
            edges.iter().map(|e| e.generated_path.as_str()).collect();
        assert!(gen_paths.contains("services/auth/gen/user.pb.go"));
        assert!(gen_paths.contains("services/auth/gen/user_pb2.py"));
        assert!(!gen_paths.contains("services/billing/gen/user.pb.go"));
    }

    #[test]
    fn test_detect_boundary_edges_dep_scope() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::create_schema_for_test(&conn);

        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('proto-pkg', 'proto', 'proto'), ('consumer', 'apps/consumer', 'go')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES ('consumer', 'proto-pkg', 'runtime', 1)",
            [],
        )
        .unwrap();

        let files: Vec<(String, Option<String>, String, u64)> = vec![
            (
                "proto/api.proto".into(),
                Some("proto-pkg".into()),
                "proto".into(),
                100,
            ),
            (
                "apps/consumer/api.pb.go".into(),
                Some("consumer".into()),
                "go".into(),
                200,
            ),
        ];

        let edges = detect_boundary_edges(&conn, &files).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].generated_package.as_deref(), Some("consumer"));
    }
}
