use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Serialize)]
pub struct PackageRow {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DependencyRow {
    pub package: String,
    pub dependency: String,
    pub dep_kind: String,
    pub version_req: Option<String>,
    pub is_internal: bool,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub dep_kind: String,
}

#[derive(Debug, Serialize)]
pub struct IndexStatus {
    pub indexed_at: Option<String>,
    pub git_commit: Option<String>,
    pub package_count: Option<String>,
    pub symbol_count: Option<String>,
    pub reference_count: Option<String>,
    pub file_count: Option<String>,
    pub doc_count: Option<String>,
    pub total_duration_ms: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
    pub package: String,
    pub file_path: String,
    pub line: i64,
    pub visibility: String,
    pub parent_symbol: Option<String>,
    pub return_type: Option<String>,
    pub parameters: Option<String>,
}

/// FTS5 search across symbol names and signatures.
pub fn search_symbols(
    conn: &Connection,
    query: &str,
    package_filter: Option<&str>,
    kind_filter: Option<&str>,
    limit: u32,
) -> Result<Vec<SymbolRow>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let sanitized = format!("\"{}\"", query.replace('"', "\"\""));
    let limit = limit.min(200) as i64;

    // For kind-filtered queries, push the kind filter into FTS MATCH using column syntax.
    // This lets FTS5 filter at the index level instead of post-filtering via JOIN.
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match (package_filter, kind_filter) {
            (Some(pkg), Some(kind)) => {
                let fts_query = format!("{} kind:\"{}\"", sanitized, kind.replace('"', "\"\""));
                (
                    "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1 AND s.package = ?2
             ORDER BY rank
             LIMIT ?3",
                    vec![
                        Box::new(fts_query) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(pkg.to_string()),
                        Box::new(limit),
                    ],
                )
            }
            (Some(pkg), None) => (
                "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1 AND s.package = ?2
             ORDER BY rank
             LIMIT ?3",
                vec![
                    Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(pkg.to_string()),
                    Box::new(limit),
                ],
            ),
            (None, Some(kind)) => {
                let fts_query = format!("{} kind:\"{}\"", sanitized, kind.replace('"', "\"\""));
                (
                    "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
                    vec![
                        Box::new(fts_query) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(limit),
                    ],
                )
            }
            (None, None) => (
                "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
                vec![
                    Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit),
                ],
            ),
        };

    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(SymbolRow {
            name: row.get(0)?,
            kind: row.get(1)?,
            signature: row.get(2)?,
            package: row.get(3)?,
            file_path: row.get(4)?,
            line: row.get(5)?,
            visibility: row.get(6)?,
            parent_symbol: row.get(7)?,
            return_type: row.get(8)?,
            parameters: row.get(9)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// List all symbols in a package, optionally filtered by kind.
pub fn get_package_symbols(
    conn: &Connection,
    package: &str,
    kind_filter: Option<&str>,
) -> Result<Vec<SymbolRow>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match kind_filter {
        Some(kind) => (
            "SELECT name, kind, signature, package, file_path, line,
                    visibility, parent_symbol, return_type, parameters
             FROM symbols
             WHERE package = ?1 AND kind = ?2
             ORDER BY file_path, line",
            vec![Box::new(package.to_string()), Box::new(kind.to_string())],
        ),
        None => (
            "SELECT name, kind, signature, package, file_path, line,
                    visibility, parent_symbol, return_type, parameters
             FROM symbols
             WHERE package = ?1
             ORDER BY file_path, line",
            vec![Box::new(package.to_string())],
        ),
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(SymbolRow {
            name: row.get(0)?,
            kind: row.get(1)?,
            signature: row.get(2)?,
            package: row.get(3)?,
            file_path: row.get(4)?,
            line: row.get(5)?,
            visibility: row.get(6)?,
            parent_symbol: row.get(7)?,
            return_type: row.get(8)?,
            parameters: row.get(9)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// List all symbols defined in a specific file, optionally filtered by kind.
pub fn get_file_symbols(
    conn: &Connection,
    file_path: &str,
    kind_filter: Option<&str>,
) -> Result<Vec<SymbolRow>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match kind_filter {
        Some(kind) => (
            "SELECT name, kind, signature, package, file_path, line,
                    visibility, parent_symbol, return_type, parameters
             FROM symbols
             WHERE file_path = ?1 AND kind = ?2
             ORDER BY line",
            vec![Box::new(file_path.to_string()), Box::new(kind.to_string())],
        ),
        None => (
            "SELECT name, kind, signature, package, file_path, line,
                    visibility, parent_symbol, return_type, parameters
             FROM symbols
             WHERE file_path = ?1
             ORDER BY line",
            vec![Box::new(file_path.to_string())],
        ),
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(SymbolRow {
            name: row.get(0)?,
            kind: row.get(1)?,
            signature: row.get(2)?,
            package: row.get(3)?,
            file_path: row.get(4)?,
            line: row.get(5)?,
            visibility: row.get(6)?,
            parent_symbol: row.get(7)?,
            return_type: row.get(8)?,
            parameters: row.get(9)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[derive(Debug, Serialize)]
pub struct FileRow {
    pub path: String,
    pub package: Option<String>,
    pub extension: String,
    pub size_bytes: i64,
}

/// FTS5 search across file paths. Returns up to 20 results.
pub fn search_files(
    conn: &Connection,
    query: &str,
    package_filter: Option<&str>,
    extension_filter: Option<&str>,
) -> Result<Vec<FileRow>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let sanitized = format!("\"{}\"", query.replace('"', "\"\""));

    let limit = 20i64;

    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) =
        match (package_filter, extension_filter) {
            (Some(pkg), Some(ext)) => (
                "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.package = ?2 AND f.extension = ?3
             ORDER BY rank
             LIMIT ?4",
                vec![
                    Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(pkg.to_string()),
                    Box::new(ext.to_string()),
                    Box::new(limit),
                ],
            ),
            (Some(pkg), None) => (
                "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.package = ?2
             ORDER BY rank
             LIMIT ?3",
                vec![
                    Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(pkg.to_string()),
                    Box::new(limit),
                ],
            ),
            (None, Some(ext)) => (
                "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.extension = ?2
             ORDER BY rank
             LIMIT ?3",
                vec![
                    Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(ext.to_string()),
                    Box::new(limit),
                ],
            ),
            (None, None) => (
                "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
                vec![
                    Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit),
                ],
            ),
        };

    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(FileRow {
            path: row.get(0)?,
            package: row.get(1)?,
            extension: row.get(2)?,
            size_bytes: row.get(3)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// List all files belonging to a package, optionally filtered by extension. Ordered by path.
pub fn list_package_files(
    conn: &Connection,
    package: &str,
    extension_filter: Option<&str>,
) -> Result<Vec<FileRow>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match extension_filter {
        Some(ext) => (
            "SELECT path, package, extension, size_bytes
             FROM files
             WHERE package = ?1 AND extension = ?2
             ORDER BY path",
            vec![Box::new(package.to_string()), Box::new(ext.to_string())],
        ),
        None => (
            "SELECT path, package, extension, size_bytes
             FROM files
             WHERE package = ?1
             ORDER BY path",
            vec![Box::new(package.to_string())],
        ),
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(FileRow {
            path: row.get(0)?,
            package: row.get(1)?,
            extension: row.get(2)?,
            size_bytes: row.get(3)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// FTS5 search across package name, description, and path.
pub fn search_packages(conn: &Connection, query: &str, limit: u32) -> Result<Vec<PackageRow>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Sanitize for FTS5: wrap in double quotes, escape internal quotes
    let sanitized = format!("\"{}\"", query.replace('"', "\"\""));
    let limit = limit.min(200) as i64;
    let mut stmt = conn.prepare_cached(
        "SELECT p.name, p.path, p.kind, p.version, p.description, p.metadata
         FROM packages_fts f
         JOIN packages p ON p.rowid = f.rowid
         WHERE packages_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![&sanitized, &limit], |row| {
        Ok(PackageRow {
            name: row.get(0)?,
            path: row.get(1)?,
            kind: row.get(2)?,
            version: row.get(3)?,
            description: row.get(4)?,
            metadata: row.get(5)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Exact name lookup for a single package.
#[allow(dead_code)]
pub fn get_package(conn: &Connection, name: &str) -> Result<Option<PackageRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT name, path, kind, version, description, metadata
         FROM packages
         WHERE name = ?1",
    )?;
    let mut rows = stmt.query_map([name], |row| {
        Ok(PackageRow {
            name: row.get(0)?,
            path: row.get(1)?,
            kind: row.get(2)?,
            version: row.get(3)?,
            description: row.get(4)?,
            metadata: row.get(5)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// List dependencies of a given package. When `internal_only` is true, only
/// returns dependencies where `is_internal = 1`.
pub fn package_dependencies(
    conn: &Connection,
    name: &str,
    internal_only: bool,
) -> Result<Vec<DependencyRow>> {
    let sql = if internal_only {
        "SELECT package, dependency, dep_kind, version_req, is_internal
         FROM dependencies
         WHERE package = ?1 AND is_internal = 1"
    } else {
        "SELECT package, dependency, dep_kind, version_req, is_internal
         FROM dependencies
         WHERE package = ?1"
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([name], |row| {
        Ok(DependencyRow {
            package: row.get(0)?,
            dependency: row.get(1)?,
            dep_kind: row.get(2)?,
            version_req: row.get(3)?,
            is_internal: row.get::<_, i32>(4)? != 0,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Reverse dependency lookup: find all packages that depend on `name`.
pub fn package_dependents(conn: &Connection, name: &str) -> Result<Vec<DependencyRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT package, dependency, dep_kind, version_req, is_internal
         FROM dependencies
         WHERE dependency = ?1",
    )?;
    let rows = stmt.query_map([name], |row| {
        Ok(DependencyRow {
            package: row.get(0)?,
            dependency: row.get(1)?,
            dep_kind: row.get(2)?,
            version_req: row.get(3)?,
            is_internal: row.get::<_, i32>(4)? != 0,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// BFS traversal of the dependency graph starting from `root`, up to `max_depth` levels.
/// When `internal_only` is true, only follows internal dependency edges.
/// Returns at most `MAX_EDGES` edges to prevent unbounded memory growth in large monorepos.
pub fn dependency_graph(
    conn: &Connection,
    root: &str,
    max_depth: u32,
    internal_only: bool,
) -> Result<Vec<GraphEdge>> {
    const MAX_EDGES: usize = 10_000;

    let sql = if internal_only {
        "SELECT dependency, dep_kind FROM dependencies WHERE package = ?1 AND is_internal = 1"
    } else {
        "SELECT dependency, dep_kind FROM dependencies WHERE package = ?1"
    };

    let mut edges = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();

    visited.insert(root.to_string());
    queue.push_back((root.to_string(), 0));

    let mut stmt = conn.prepare_cached(sql)?;

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth || edges.len() >= MAX_EDGES {
            continue;
        }
        let rows = stmt.query_map([&current], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (dep, kind) = row?;
            edges.push(GraphEdge {
                from: current.clone(),
                to: dep.clone(),
                dep_kind: kind,
            });
            if edges.len() >= MAX_EDGES {
                break;
            }
            if visited.insert(dep.clone()) {
                queue.push_back((dep, depth + 1));
            }
        }
    }

    Ok(edges)
}

/// List all packages, optionally filtered by kind (e.g. "npm", "go").
pub fn list_packages(conn: &Connection, kind: Option<&str>) -> Result<Vec<PackageRow>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match kind {
        Some(k) => (
            "SELECT name, path, kind, version, description, metadata
             FROM packages
             WHERE kind = ?1
             ORDER BY name",
            vec![Box::new(k.to_string())],
        ),
        None => (
            "SELECT name, path, kind, version, description, metadata
             FROM packages
             ORDER BY name",
            vec![],
        ),
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(PackageRow {
            name: row.get(0)?,
            path: row.get(1)?,
            kind: row.get(2)?,
            version: row.get(3)?,
            description: row.get(4)?,
            metadata: row.get(5)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Read indexing status from the shire_meta table.
pub fn index_status(conn: &Connection) -> Result<IndexStatus> {
    let get_meta = |key: &str| -> Result<Option<String>> {
        let mut stmt = conn.prepare_cached("SELECT value FROM shire_meta WHERE key = ?1")?;
        let mut rows = stmt.query_map([key], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(val) => Ok(Some(val?)),
            None => Ok(None),
        }
    };

    Ok(IndexStatus {
        indexed_at: get_meta("indexed_at")?,
        git_commit: get_meta("git_commit")?,
        package_count: get_meta("package_count")?,
        symbol_count: get_meta("symbol_count")?,
        reference_count: get_meta("reference_count")?,
        file_count: get_meta("file_count")?,
        doc_count: get_meta("doc_count")?,
        total_duration_ms: get_meta("total_duration_ms")?,
    })
}

/// BFS traversal of the reverse dependency graph starting from `root`, up to `max_depth` levels.
/// Finds all packages that transitively depend on `root`.
/// Returns at most `MAX_EDGES` edges to prevent unbounded memory growth.
#[allow(dead_code)]
pub fn reverse_dependency_graph(
    conn: &Connection,
    root: &str,
    max_depth: u32,
) -> Result<Vec<GraphEdge>> {
    const MAX_EDGES: usize = 10_000;

    let sql =
        "SELECT package, dep_kind FROM dependencies WHERE dependency = ?1 AND is_internal = 1";

    let mut edges = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();

    visited.insert(root.to_string());
    queue.push_back((root.to_string(), 0));

    let mut stmt = conn.prepare_cached(sql)?;

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth || edges.len() >= MAX_EDGES {
            continue;
        }
        let rows = stmt.query_map([&current], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (dependent, kind) = row?;
            edges.push(GraphEdge {
                from: dependent.clone(),
                to: current.clone(),
                dep_kind: kind,
            });
            if edges.len() >= MAX_EDGES {
                break;
            }
            if visited.insert(dependent.clone()) {
                queue.push_back((dependent, depth + 1));
            }
        }
    }

    Ok(edges)
}

#[derive(Debug, Serialize)]
pub struct DocRow {
    pub path: String,
    pub package: Option<String>,
    pub title: Option<String>,
    pub snippet: String,
    pub size_bytes: i64,
}

/// FTS5 search across documentation content (title, body, path).
pub fn search_docs(
    conn: &Connection,
    query: &str,
    package_filter: Option<&str>,
    limit: u32,
) -> Result<Vec<DocRow>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let sanitized = format!("\"{}\"", query.replace('"', "\"\""));
    let limit = limit.min(200) as i64;

    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match package_filter {
        Some(pkg) => (
            "SELECT d.path, d.package, d.title,
                    snippet(docs_fts, 1, '**', '**', '…', 40) AS snippet,
                    d.size_bytes
             FROM docs_fts f
             JOIN docs d ON d.rowid = f.rowid
             WHERE docs_fts MATCH ?1 AND d.package = ?2
             ORDER BY rank
             LIMIT ?3",
            vec![
                Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                Box::new(pkg.to_string()),
                Box::new(limit),
            ],
        ),
        None => (
            "SELECT d.path, d.package, d.title,
                    snippet(docs_fts, 1, '**', '**', '…', 40) AS snippet,
                    d.size_bytes
             FROM docs_fts f
             JOIN docs d ON d.rowid = f.rowid
             WHERE docs_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
            vec![
                Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>,
                Box::new(limit),
            ],
        ),
    };

    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(DocRow {
            path: row.get(0)?,
            package: row.get(1)?,
            title: row.get(2)?,
            snippet: row.get(3)?,
            size_bytes: row.get(4)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

use crate::symbols::ReferenceInfo;
use std::collections::HashMap;

/// Build a `file_path → files.id` lookup map from the `files` table. Callers
/// pass this to `batch_insert_references` so references can be inserted with
/// their compact `file_id` instead of repeated path strings.
///
/// This is intentionally lazy: we start empty and resolve/cache IDs on demand
/// in `batch_insert_references`. Preloading the full `files` table can spike
/// memory on large repos with hundreds of thousands of file rows.
pub fn build_file_id_map(conn: &Connection) -> Result<HashMap<String, i64>> {
    let _ = conn;
    Ok(HashMap::new())
}

/// Insert a batch of `ReferenceInfo` rows into `symbol_refs`.
///
/// Uses multi-row INSERT batching (128 rows per prepared-statement execution)
/// to amortize per-call overhead. No FTS triggers fire — `symbol_refs` has no
/// FTS virtual table; all MCP queries use exact-name B-tree lookups.
///
/// `file_ids` should contain an entry for every `r.file_path` that appears in
/// `refs`. In normal operation `phase_index_files` runs before symbol
/// extraction, so every source path we extract from is already in `files`.
/// A handful of paths (e.g. dotfiles like `.eslintrc.js`) can slip past the
/// file walker but still produce refs — for those, this function inserts a
/// `files` row on the fly and updates `file_ids` so repeated paths are
/// resolved from the cache.
///
/// For large bulk inserts, callers should drop the `symbol_refs` B-tree
/// indexes first (via `db::drop_symbol_refs_indexes`) and recreate them
/// afterward (`db::recreate_symbol_refs_indexes`) — the per-row B-tree
/// updates dominate otherwise.
///
/// Callers are expected to wrap the surrounding work in a transaction for
/// throughput; this function does not open one itself.
pub fn batch_insert_references(
    conn: &Connection,
    package: Option<&str>,
    refs: &[ReferenceInfo],
    file_ids: &mut HashMap<String, i64>,
) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }

    // Resolve file_id for every ref up front. For paths not already in the
    // map, insert a `files` row (pkg=None, ext derived from suffix) and cache
    // the new id.
    //
    // The backfill is a safety net for paths that the file walker skipped
    // but the symbol extractor reached (e.g. dotfiles, symlinks, walker
    // config mismatch). Losing refs would be worse than a `files` row with
    // `package=NULL, size_bytes=0`, so we keep the insert — but we log at
    // WARN so operators can audit and the two walkers can be brought into
    // alignment.
    let mut resolved: Vec<(&ReferenceInfo, i64)> = Vec::with_capacity(refs.len());
    let mut lookup_stmt = conn.prepare_cached("SELECT id FROM files WHERE path = ?1")?;
    for r in refs {
        let path = r.file_path.as_ref();
        let id = match file_ids.get(path) {
            Some(&id) => id,
            None => match lookup_stmt.query_row([path], |row| row.get::<_, i64>(0)) {
                Ok(id) => {
                    file_ids.insert(path.to_string(), id);
                    id
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    let ext = path
                        .rsplit_once('.')
                        .map(|(_, e)| e.to_lowercase())
                        .unwrap_or_default();
                    tracing::warn!(
                        path,
                        package,
                        "batch_insert_references: file_id missing, synthesizing files row \
                             (file walker and symbol extractor are not aligned for this path)"
                    );
                    conn.execute(
                            "INSERT OR IGNORE INTO files (path, package, extension, size_bytes) VALUES (?1, NULL, ?2, 0)",
                            rusqlite::params![path, ext],
                        )?;
                    let id = lookup_stmt.query_row([path], |row| row.get::<_, i64>(0))?;
                    file_ids.insert(path.to_string(), id);
                    id
                }
                Err(e) => return Err(e.into()),
            },
        };
        resolved.push((r, id));
    }

    if resolved.is_empty() {
        return Ok(());
    }

    // Multi-row INSERT batching: group rows into chunks and bind many rows per
    // statement. SQLite's default SQLITE_MAX_VARIABLE_NUMBER is 32766 (modern
    // builds). With 6 columns per row, a chunk of 128 rows uses 768 parameters
    // — well under the limit — and collapses ~520k per-statement calls into
    // ~4k prepared-statement executions.
    const ROWS_PER_CHUNK: usize = 128;
    const COLS_PER_ROW: usize = 6;

    // Build the multi-row placeholder template once (reused across full chunks)
    let full_chunk_sql = build_multi_row_insert_sql(ROWS_PER_CHUNK, COLS_PER_ROW);
    let mut full_stmt = conn.prepare_cached(&full_chunk_sql)?;

    let mut iter = resolved.chunks_exact(ROWS_PER_CHUNK);
    for chunk in &mut iter {
        bind_and_execute_chunk(&mut full_stmt, chunk, package)?;
    }

    // Handle the tail (rows that didn't fit a full chunk) with a sized statement.
    let remainder = iter.remainder();
    if !remainder.is_empty() {
        let tail_sql = build_multi_row_insert_sql(remainder.len(), COLS_PER_ROW);
        let mut tail_stmt = conn.prepare_cached(&tail_sql)?;
        bind_and_execute_chunk(&mut tail_stmt, remainder, package)?;
    }

    Ok(())
}

/// Bind and execute one chunk of the multi-row INSERT. Uses
/// `raw_bind_parameter` to avoid building a `Vec<Box<dyn ToSql>>` for each
/// chunk — the Box+dyn pair was ~768 heap allocations per 128-row chunk
/// (6 columns × 128 rows), and the per-row `r.name.clone()` was another
/// 128. At 5M refs, that was on the order of 40M needless heap allocations
/// inside the bulk insert path.
fn bind_and_execute_chunk(
    stmt: &mut rusqlite::CachedStatement<'_>,
    chunk: &[(&ReferenceInfo, i64)],
    package: Option<&str>,
) -> Result<()> {
    let mut col = 1;
    for (r, file_id) in chunk {
        stmt.raw_bind_parameter(col, r.name.as_str())?;
        stmt.raw_bind_parameter(col + 1, r.kind.as_str())?;
        stmt.raw_bind_parameter(col + 2, *file_id)?;
        stmt.raw_bind_parameter(col + 3, r.line as i64)?;
        stmt.raw_bind_parameter(col + 4, package)?;
        stmt.raw_bind_parameter(col + 5, r.enclosing_symbol.as_deref())?;
        col += 6;
    }
    stmt.raw_execute()?;
    Ok(())
}

fn build_multi_row_insert_sql(rows: usize, cols: usize) -> String {
    let mut sql = String::from(
        "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) VALUES ",
    );
    for r in 0..rows {
        if r > 0 {
            sql.push(',');
        }
        sql.push('(');
        for c in 0..cols {
            if c > 0 {
                sql.push(',');
            }
            sql.push('?');
        }
        sql.push(')');
    }
    sql
}

/// Delete all rows in `symbol_refs` for a given file path. Used during
/// file-granularity incremental rebuild before inserting fresh references.
/// Resolves `file_path → file_id` via the `files` table.
pub fn delete_references_for_file(conn: &Connection, file_path: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM symbol_refs WHERE file_id = (SELECT id FROM files WHERE path = ?1)",
        [file_path],
    )?;
    Ok(())
}

/// Delete all rows in `symbol_refs` for a given package. Used during
/// package-level rebuilds. Relies on the `(package, name)` leading-column
/// index (`idx_refs_package_name`).
pub fn delete_references_for_package(conn: &Connection, package: &str) -> Result<()> {
    conn.execute("DELETE FROM symbol_refs WHERE package = ?1", [package])?;
    Ok(())
}

// ── Cross-reference queries ──────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReferenceRow {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: i64,
    pub package: Option<String>,
    pub enclosing_symbol: Option<String>,
}

/// Collect a rusqlite row iterator into a Vec, tracing deserialization
/// failures at debug level rather than silently dropping them. Schema
/// mismatches or malformed rows would otherwise disappear behind
/// `filter_map(Result::ok)` and surface only as mysterious empty results.
fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Vec<T> {
    let mut out = Vec::new();
    for row in rows {
        match row {
            Ok(r) => out.push(r),
            Err(e) => tracing::debug!(error = %e, "skipping malformed row"),
        }
    }
    out
}

/// Builds a parameterized WHERE clause for cross-reference queries.
/// Avoids the repeated `sql.push_str(" AND col = ?")` + `params.push(Box::new(...))`
/// pattern duplicated across `query_symbol_{references,callers,callees}`.
struct RefQueryBuilder {
    sql: String,
    params: Vec<Box<dyn rusqlite::ToSql>>,
}

impl RefQueryBuilder {
    fn new(base_sql: &str, name: &str) -> Self {
        Self {
            sql: base_sql.to_string(),
            params: vec![Box::new(name.to_string())],
        }
    }

    fn filter(&mut self, column: &str, value: &str) {
        self.sql.push_str(&format!(" AND {} = ?", column));
        self.params.push(Box::new(value.to_string()));
    }

    fn build_with_order_and_limit(
        mut self,
        order_by: &str,
        limit: i64,
    ) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
        self.sql.push_str(&format!(" {order_by} LIMIT ?"));
        self.params.push(Box::new(limit));
        (self.sql, self.params)
    }
}

pub fn query_symbol_references(
    conn: &Connection,
    name: &str,
    kind: Option<&str>,
    package: Option<&str>,
    limit: i64,
) -> Result<Vec<ReferenceRow>> {
    let mut qb = RefQueryBuilder::new(
        "SELECT r.name, r.kind, f.path, r.line, r.package, r.enclosing_symbol \
         FROM symbol_refs r JOIN files f ON f.id = r.file_id WHERE r.name = ?",
        name,
    );
    if let Some(k) = kind {
        qb.filter("r.kind", k);
    }
    if let Some(p) = package {
        qb.filter("r.package", p);
    }
    let (sql, params) = qb.build_with_order_and_limit("ORDER BY f.path, r.line", limit);
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(ReferenceRow {
            name: row.get(0)?,
            kind: row.get(1)?,
            file_path: row.get(2)?,
            line: row.get(3)?,
            package: row.get(4)?,
            enclosing_symbol: row.get(5)?,
        })
    })?;
    Ok(collect_rows(rows))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CallerRow {
    pub caller_name: String,
    pub caller_file: String,
    pub caller_line: i64,
    pub caller_package: Option<String>,
    pub call_sites: i64,
}

pub fn query_symbol_callers(
    conn: &Connection,
    name: &str,
    package: Option<&str>,
    limit: i64,
) -> Result<Vec<CallerRow>> {
    // Aggregate over `symbol_refs` first (grouping by `file_id`, not joined
    // path text), then join to `files` for display fields.
    let mut sql = String::from(
        "SELECT g.enclosing_symbol, f.path, g.first_line, g.package, g.call_sites \
         FROM ( \
             SELECT r.enclosing_symbol, r.file_id, MIN(r.line) AS first_line, r.package, COUNT(*) AS call_sites \
             FROM symbol_refs r \
             WHERE r.name = ? AND r.kind = 'call' AND r.enclosing_symbol IS NOT NULL",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(name.to_string())];
    if let Some(p) = package {
        sql.push_str(" AND r.package = ?");
        params.push(Box::new(p.to_string()));
    }
    sql.push_str(
        " GROUP BY r.enclosing_symbol, r.file_id, r.package \
         ) AS g \
         JOIN files f ON f.id = g.file_id \
         ORDER BY g.call_sites DESC, g.enclosing_symbol ASC \
         LIMIT ?",
    );
    params.push(Box::new(limit));

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(CallerRow {
            caller_name: row.get(0)?,
            caller_file: row.get(1)?,
            caller_line: row.get(2)?,
            caller_package: row.get(3)?,
            call_sites: row.get(4)?,
        })
    })?;
    Ok(collect_rows(rows))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CalleeRow {
    pub callee_name: String,
    pub first_file: String,
    pub first_line: i64,
    pub call_sites: i64,
}

pub fn query_symbol_callees(
    conn: &Connection,
    enclosing: &str,
    package: Option<&str>,
    limit: i64,
) -> Result<Vec<CalleeRow>> {
    // Same shape as callers: aggregate within `symbol_refs` using `file_id`
    // so grouping/sorting can ride ref-table indexes before joining `files`.
    let mut sql = String::from(
        "SELECT g.name, f.path, g.first_line, g.call_sites \
         FROM ( \
             SELECT r.name, r.file_id, MIN(r.line) AS first_line, COUNT(*) AS call_sites \
             FROM symbol_refs r \
             WHERE r.enclosing_symbol = ? AND r.kind = 'call'",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(enclosing.to_string())];
    if let Some(p) = package {
        sql.push_str(" AND r.package = ?");
        params.push(Box::new(p.to_string()));
    }
    sql.push_str(
        " GROUP BY r.name, r.file_id \
         ) AS g \
         JOIN files f ON f.id = g.file_id \
         ORDER BY g.call_sites DESC, g.name ASC \
         LIMIT ?",
    );
    params.push(Box::new(limit));

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(CalleeRow {
            callee_name: row.get(0)?,
            first_file: row.get(1)?,
            first_line: row.get(2)?,
            call_sites: row.get(3)?,
        })
    })?;
    Ok(collect_rows(rows))
}

// ── Change impact analysis ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TransitiveImpact {
    /// The package affected transitively.
    pub package: String,
    /// A directly-affected package that `package` depends on (the edge's
    /// downstream end). When multiple paths exist, the shortest is reported.
    pub via: String,
    pub dep_kind: String,
    /// Hops from an affected package. 1 = direct reverse-dep of an affected
    /// package; 2 = depends on a reverse-dep; etc.
    pub distance: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeImpactSummary {
    /// Total same-package refs (may exceed `direct_impact.len()` when the
    /// returned rows were truncated to `per_bucket_limit`).
    pub direct_count: usize,
    /// Total cross-package refs (may exceed `cross_package_impact.len()`
    /// when the returned rows were truncated to `per_bucket_limit`).
    pub cross_package_count: usize,
    /// Unique packages that contain cross-package references. Computed from
    /// the full ref set before truncation — this is the authoritative list
    /// of directly affected packages.
    pub affected_packages: Vec<String>,
    pub transitive_package_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeImpact {
    pub symbol: String,
    /// The package where the symbol is defined. `None` when the symbol is not
    /// in the `symbols` table and no `package` hint was given — in that case
    /// every ref falls into `cross_package_impact`.
    pub home_package: Option<String>,
    pub direct_impact: Vec<ReferenceRow>,
    pub cross_package_impact: Vec<ReferenceRow>,
    pub transitive_impact: Vec<TransitiveImpact>,
    pub summary: ChangeImpactSummary,
}

/// Resolve the "home package" of a symbol — the package that defines it.
/// Used by `change_impact` to decide which refs are in-package (direct) vs
/// cross-package. Picks the first match when multiple same-name symbols exist
/// across packages (callers can disambiguate by passing `package` explicitly).
fn resolve_home_package(conn: &Connection, name: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare_cached("SELECT package FROM symbols WHERE name = ?1 ORDER BY package LIMIT 1")?;
    let mut rows = stmt.query_map([name], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Compute the transitive impact of changing a symbol by combining the
/// cross-reference index with the dependency graph.
///
/// - Partitions refs into same-package (`direct_impact`) vs other-package
///   (`cross_package_impact`) using `home_package` (caller-supplied or looked
///   up from `symbols`).
/// - BFS the reverse-dep graph starting from packages with cross-package refs,
///   up to `transitive_depth`. Packages already in `affected_packages` are
///   excluded — they would overstate impact.
pub fn change_impact(
    conn: &Connection,
    name: &str,
    package_hint: Option<&str>,
    transitive_depth: u32,
    per_bucket_limit: i64,
) -> Result<ChangeImpact> {
    // Fetch all refs with a high safety cap, then partition. Using
    // per_bucket_limit at fetch time would starve a bucket: refs are
    // ordered by (file_path, line), so if one side's paths sort first,
    // the other bucket gets zero rows — affected_packages is incomplete
    // and BFS under-reports blast radius. The safety cap keeps memory
    // bounded for pathologically-called symbols.
    const MAX_REFS_SCANNED: i64 = 10_000;
    let all_refs = query_symbol_references(conn, name, None, None, MAX_REFS_SCANNED)?;

    let home_package = match package_hint {
        Some(p) => Some(p.to_string()),
        None => resolve_home_package(conn, name)?,
    };

    let mut direct_impact: Vec<ReferenceRow> = Vec::new();
    let mut cross_package_impact: Vec<ReferenceRow> = Vec::new();
    let mut affected_packages_set: HashSet<String> = HashSet::new();

    for r in all_refs {
        match (&home_package, &r.package) {
            (Some(home), Some(pkg)) if pkg == home => direct_impact.push(r),
            (_, Some(pkg)) => {
                affected_packages_set.insert(pkg.clone());
                cross_package_impact.push(r);
            }
            // Refs with NULL package (ref was extracted but package couldn't
            // be attributed — rare, usually means file wasn't mapped to a
            // package). Treat as cross-package since we can't prove same-pkg.
            (_, None) => cross_package_impact.push(r),
        }
    }

    // Capture true counts before truncation — users need to see the real
    // blast radius even when we cap the returned rows for display.
    let direct_count = direct_impact.len();
    let cross_package_count = cross_package_impact.len();

    direct_impact.truncate(per_bucket_limit as usize);
    cross_package_impact.truncate(per_bucket_limit as usize);

    // BFS reverse-dep graph from each affected package. `via` records the
    // first affected package we reached this transitive package through.
    // `is_internal = 1` matches reverse_dependency_graph() — external deps
    // that share a name with an internal package must not appear here.
    let transitive_cap = per_bucket_limit as usize;
    let mut transitive_impact: Vec<TransitiveImpact> = Vec::new();
    if transitive_depth > 0 && !affected_packages_set.is_empty() {
        let mut visited: HashSet<String> = affected_packages_set.clone();
        // Also don't recurse back into the home package.
        if let Some(ref home) = home_package {
            visited.insert(home.clone());
        }
        let mut queue: VecDeque<(String, String, u32)> = VecDeque::new();
        for pkg in &affected_packages_set {
            queue.push_back((pkg.clone(), pkg.clone(), 0));
        }

        let mut stmt = conn.prepare_cached(
            "SELECT package, dep_kind FROM dependencies WHERE dependency = ?1 AND is_internal = 1",
        )?;

        while let Some((current, origin, depth)) = queue.pop_front() {
            if depth >= transitive_depth || transitive_impact.len() >= transitive_cap {
                continue;
            }
            let rows = stmt.query_map([&current], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (dependent, dep_kind) = row?;
                if !visited.insert(dependent.clone()) {
                    continue;
                }
                transitive_impact.push(TransitiveImpact {
                    package: dependent.clone(),
                    via: origin.clone(),
                    dep_kind,
                    distance: depth + 1,
                });
                if transitive_impact.len() >= transitive_cap {
                    break;
                }
                queue.push_back((dependent, origin.clone(), depth + 1));
            }
        }
    }

    let mut affected_packages: Vec<String> = affected_packages_set.into_iter().collect();
    affected_packages.sort();

    let summary = ChangeImpactSummary {
        direct_count,
        cross_package_count,
        affected_packages,
        transitive_package_count: transitive_impact.len(),
    };

    Ok(ChangeImpact {
        symbol: name.to_string(),
        home_package,
        direct_impact,
        cross_package_impact,
        transitive_impact,
        summary,
    })
}

// ── Boundary edge queries ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BoundaryEdge {
    pub source_path: String,
    pub generated_path: String,
    pub source_package: Option<String>,
    pub generated_package: Option<String>,
    pub kind: String,
}

pub fn batch_insert_boundary_edges(conn: &Connection, edges: &[BoundaryEdge]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO boundary_edges \
         (source_path, generated_path, source_package, generated_package, kind) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for e in edges {
        stmt.execute(rusqlite::params![
            e.source_path,
            e.generated_path,
            e.source_package,
            e.generated_package,
            e.kind,
        ])?;
    }
    Ok(())
}

pub fn clear_boundary_edges(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM boundary_edges", [])?;
    Ok(())
}

pub fn query_schema_consumers(conn: &Connection, source_path: &str) -> Result<Vec<BoundaryEdge>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_path, generated_path, source_package, generated_package, kind \
         FROM boundary_edges WHERE source_path = ?1 ORDER BY generated_path",
    )?;
    let rows = stmt.query_map([source_path], |row| {
        Ok(BoundaryEdge {
            source_path: row.get(0)?,
            generated_path: row.get(1)?,
            source_package: row.get(2)?,
            generated_package: row.get(3)?,
            kind: row.get(4)?,
        })
    })?;
    Ok(collect_rows(rows))
}

pub fn query_generated_from(conn: &Connection, generated_path: &str) -> Result<Vec<BoundaryEdge>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_path, generated_path, source_package, generated_package, kind \
         FROM boundary_edges WHERE generated_path = ?1 ORDER BY source_path",
    )?;
    let rows = stmt.query_map([generated_path], |row| {
        Ok(BoundaryEdge {
            source_path: row.get(0)?,
            generated_path: row.get(1)?,
            source_package: row.get(2)?,
            generated_package: row.get(3)?,
            kind: row.get(4)?,
        })
    })?;
    Ok(collect_rows(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_schema_for_test;
    use rusqlite::Connection;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        create_schema_for_test(&conn);
        seed_test_data(&conn);
        conn
    }

    fn seed_test_data(conn: &Connection) {
        // 3 packages
        conn.execute(
            "INSERT INTO packages (name, path, kind, version, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("auth-service", "services/auth", "npm", "1.0.0", "Authentication and authorization service"),
        ).unwrap();
        conn.execute(
            "INSERT INTO packages (name, path, kind, version, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("shared-types", "packages/shared-types", "npm", "0.5.0", "Shared TypeScript type definitions"),
        ).unwrap();
        conn.execute(
            "INSERT INTO packages (name, path, kind, version, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("api-gateway", "services/gateway", "go", "2.1.0", "API gateway and routing layer"),
        ).unwrap();

        // Dependency edges:
        //   api-gateway -> auth-service (internal, runtime)
        //   auth-service -> shared-types (internal, runtime)
        //   auth-service -> express (external, runtime)
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, version_req, is_internal) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("api-gateway", "auth-service", "runtime", None::<String>, 1),
        ).unwrap();
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, version_req, is_internal) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("auth-service", "shared-types", "runtime", "^0.5.0", 1),
        ).unwrap();
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, version_req, is_internal) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("auth-service", "express", "runtime", "^4.18.0", 0),
        ).unwrap();

        // Meta
        conn.execute(
            "INSERT INTO shire_meta (key, value) VALUES (?1, ?2)",
            ("indexed_at", "2026-02-25T10:00:00Z"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO shire_meta (key, value) VALUES (?1, ?2)",
            ("git_commit", "abc123"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO shire_meta (key, value) VALUES (?1, ?2)",
            ("package_count", "3"),
        )
        .unwrap();
    }

    #[test]
    fn test_search_packages_finds_by_name() {
        let conn = test_db();
        let results = search_packages(&conn, "auth", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "auth-service");
    }

    #[test]
    fn test_search_packages_finds_by_description() {
        let conn = test_db();
        let results = search_packages(&conn, "TypeScript", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "shared-types");
    }

    #[test]
    fn test_search_packages_no_match() {
        let conn = test_db();
        let results = search_packages(&conn, "nonexistent", 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_package_existing() {
        let conn = test_db();
        let pkg = get_package(&conn, "auth-service").unwrap();
        assert!(pkg.is_some());
        let pkg = pkg.unwrap();
        assert_eq!(pkg.name, "auth-service");
        assert_eq!(pkg.path, "services/auth");
        assert_eq!(pkg.kind, "npm");
        assert_eq!(pkg.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn test_get_package_nonexistent() {
        let conn = test_db();
        let pkg = get_package(&conn, "does-not-exist").unwrap();
        assert!(pkg.is_none());
    }

    #[test]
    fn test_package_dependencies_all() {
        let conn = test_db();
        let deps = package_dependencies(&conn, "auth-service", false).unwrap();
        assert_eq!(deps.len(), 2);
        let dep_names: Vec<&str> = deps.iter().map(|d| d.dependency.as_str()).collect();
        assert!(dep_names.contains(&"shared-types"));
        assert!(dep_names.contains(&"express"));
    }

    #[test]
    fn test_package_dependencies_internal_only() {
        let conn = test_db();
        let deps = package_dependencies(&conn, "auth-service", true).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].dependency, "shared-types");
        assert!(deps[0].is_internal);
    }

    #[test]
    fn test_package_dependents() {
        let conn = test_db();
        let dependents = package_dependents(&conn, "auth-service").unwrap();
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0].package, "api-gateway");
    }

    #[test]
    fn test_dependency_graph_transitive() {
        let conn = test_db();
        // api-gateway -> auth-service -> shared-types
        let edges = dependency_graph(&conn, "api-gateway", 10, true).unwrap();
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].from, "api-gateway");
        assert_eq!(edges[0].to, "auth-service");
        assert_eq!(edges[1].from, "auth-service");
        assert_eq!(edges[1].to, "shared-types");
    }

    #[test]
    fn test_dependency_graph_depth_limit() {
        let conn = test_db();
        // With max_depth=1, only one level from api-gateway
        let edges = dependency_graph(&conn, "api-gateway", 1, true).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "api-gateway");
        assert_eq!(edges[0].to, "auth-service");
    }

    #[test]
    fn test_dependency_graph_includes_external() {
        let conn = test_db();
        let edges = dependency_graph(&conn, "auth-service", 10, false).unwrap();
        assert_eq!(edges.len(), 2);
        let targets: Vec<&str> = edges.iter().map(|e| e.to.as_str()).collect();
        assert!(targets.contains(&"shared-types"));
        assert!(targets.contains(&"express"));
    }

    #[test]
    fn test_list_packages_all() {
        let conn = test_db();
        let pkgs = list_packages(&conn, None).unwrap();
        assert_eq!(pkgs.len(), 3);
        // Ordered by name
        assert_eq!(pkgs[0].name, "api-gateway");
        assert_eq!(pkgs[1].name, "auth-service");
        assert_eq!(pkgs[2].name, "shared-types");
    }

    #[test]
    fn test_list_packages_by_kind() {
        let conn = test_db();
        let npm = list_packages(&conn, Some("npm")).unwrap();
        assert_eq!(npm.len(), 2);
        let go = list_packages(&conn, Some("go")).unwrap();
        assert_eq!(go.len(), 1);
        assert_eq!(go[0].name, "api-gateway");
    }

    #[test]
    fn test_index_status() {
        let conn = test_db();
        let status = index_status(&conn).unwrap();
        assert_eq!(status.indexed_at.as_deref(), Some("2026-02-25T10:00:00Z"));
        assert_eq!(status.git_commit.as_deref(), Some("abc123"));
        assert_eq!(status.package_count.as_deref(), Some("3"));
    }

    #[test]
    fn test_index_status_empty() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema_for_test(&conn);
        let status = index_status(&conn).unwrap();
        assert!(status.indexed_at.is_none());
        assert!(status.git_commit.is_none());
        assert!(status.package_count.is_none());
    }

    fn seed_symbol_data(conn: &Connection) {
        conn.execute(
            "INSERT INTO symbols (package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, parameters)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            (
                "auth-service",
                "AuthService",
                "class",
                "export class AuthService",
                "services/auth/src/auth.ts",
                10i64,
                "public",
                None::<String>,
                None::<String>,
                None::<String>,
            ),
        ).unwrap();
        conn.execute(
            "INSERT INTO symbols (package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, parameters)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            (
                "auth-service",
                "validate",
                "method",
                "validate(token: string): Promise<boolean>",
                "services/auth/src/auth.ts",
                15i64,
                "public",
                Some("AuthService"),
                Some("Promise<boolean>"),
                Some(r#"[{"name":"token","type":"string"}]"#),
            ),
        ).unwrap();
        conn.execute(
            "INSERT INTO symbols (package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, parameters)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            (
                "shared-types",
                "UserConfig",
                "interface",
                "export interface UserConfig",
                "packages/shared-types/src/types.ts",
                5i64,
                "public",
                None::<String>,
                None::<String>,
                None::<String>,
            ),
        ).unwrap();
    }

    fn test_db_with_symbols() -> Connection {
        let conn = test_db();
        seed_symbol_data(&conn);
        conn
    }

    #[test]
    fn test_search_symbols_by_name() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "AuthService", None, None, 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "AuthService");
        assert_eq!(results[0].package, "auth-service");
    }

    #[test]
    fn test_search_symbols_by_signature() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "token", None, None, 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "validate");
    }

    #[test]
    fn test_search_symbols_filter_by_package() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "interface", Some("shared-types"), None, 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "UserConfig");

        let results = search_symbols(&conn, "interface", Some("auth-service"), None, 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_symbols_filter_by_kind() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "AuthService", None, Some("class"), 20).unwrap();
        assert_eq!(results.len(), 1);

        let results = search_symbols(&conn, "AuthService", None, Some("function"), 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_symbols_filter_by_package_and_kind() {
        let conn = test_db_with_symbols();
        // Combined filter: package + kind
        let results =
            search_symbols(&conn, "validate", Some("auth-service"), Some("method"), 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "validate");

        // Wrong package
        let results =
            search_symbols(&conn, "validate", Some("nonexistent"), Some("method"), 20).unwrap();
        assert!(results.is_empty());

        // Wrong kind
        let results =
            search_symbols(&conn, "validate", Some("auth-service"), Some("class"), 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_symbols_kind_with_special_chars() {
        let conn = test_db_with_symbols();
        // Kind filter with quotes should not cause SQL errors
        let result = search_symbols(&conn, "AuthService", None, Some("class\"test"), 20);
        assert!(result.is_ok()); // shouldn't error regardless of match
    }

    #[test]
    fn test_search_symbols_empty_query() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "", None, None, 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_package_symbols() {
        let conn = test_db_with_symbols();
        let results = get_package_symbols(&conn, "auth-service", None).unwrap();
        assert_eq!(results.len(), 2);

        let results = get_package_symbols(&conn, "auth-service", Some("method")).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "validate");
    }

    fn seed_file_data(conn: &Connection) {
        let files = vec![
            (
                "services/auth/src/auth.ts",
                Some("auth-service"),
                "ts",
                1024i64,
            ),
            (
                "services/auth/src/middleware.ts",
                Some("auth-service"),
                "ts",
                512,
            ),
            (
                "services/auth/package.json",
                Some("auth-service"),
                "json",
                256,
            ),
            (
                "packages/shared-types/src/types.ts",
                Some("shared-types"),
                "ts",
                2048,
            ),
            ("services/gateway/main.go", Some("api-gateway"), "go", 4096),
            (
                "services/gateway/handler.go",
                Some("api-gateway"),
                "go",
                3072,
            ),
            ("scripts/deploy.sh", None, "sh", 128),
            ("README.md", None, "md", 64),
        ];
        for (path, package, ext, size) in &files {
            conn.execute(
                "INSERT INTO files (path, package, extension, size_bytes) VALUES (?1, ?2, ?3, ?4)",
                (path, package, ext, size),
            )
            .unwrap();
        }
    }

    fn test_db_with_files() -> Connection {
        let conn = test_db();
        seed_file_data(&conn);
        conn
    }

    #[test]
    fn test_search_files_by_filename() {
        let conn = test_db_with_files();
        let results = search_files(&conn, "middleware", None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "services/auth/src/middleware.ts");
    }

    #[test]
    fn test_search_files_by_path_segment() {
        let conn = test_db_with_files();
        let results = search_files(&conn, "gateway", None, None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_files_filter_by_package() {
        let conn = test_db_with_files();
        let results = search_files(&conn, "ts", Some("auth-service"), None).unwrap();
        assert!(
            results
                .iter()
                .all(|f| f.package.as_deref() == Some("auth-service"))
        );
    }

    #[test]
    fn test_search_files_filter_by_extension() {
        let conn = test_db_with_files();
        let results = search_files(&conn, "auth", None, Some("ts")).unwrap();
        assert!(results.iter().all(|f| f.extension == "ts"));
    }

    #[test]
    fn test_search_files_combined_filters() {
        let conn = test_db_with_files();
        let results = search_files(&conn, "auth", Some("auth-service"), Some("ts")).unwrap();
        assert!(
            results
                .iter()
                .all(|f| f.package.as_deref() == Some("auth-service") && f.extension == "ts")
        );
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_files_empty_query() {
        let conn = test_db_with_files();
        let results = search_files(&conn, "", None, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_list_package_files_basic() {
        let conn = test_db_with_files();
        let results = list_package_files(&conn, "auth-service", None).unwrap();
        assert_eq!(results.len(), 3);
        // Should be ordered by path
        assert!(results[0].path < results[1].path);
    }

    #[test]
    fn test_list_package_files_extension_filter() {
        let conn = test_db_with_files();
        let results = list_package_files(&conn, "auth-service", Some("ts")).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|f| f.extension == "ts"));
    }

    #[test]
    fn test_reverse_dependency_graph() {
        let conn = test_db();
        // shared-types is depended on by auth-service, which is depended on by api-gateway
        let edges = reverse_dependency_graph(&conn, "shared-types", 10).unwrap();
        assert_eq!(edges.len(), 2);
        // First level: auth-service depends on shared-types
        assert_eq!(edges[0].from, "auth-service");
        assert_eq!(edges[0].to, "shared-types");
        // Second level: api-gateway depends on auth-service
        assert_eq!(edges[1].from, "api-gateway");
        assert_eq!(edges[1].to, "auth-service");
    }

    #[test]
    fn test_reverse_dependency_graph_depth_limit() {
        let conn = test_db();
        let edges = reverse_dependency_graph(&conn, "shared-types", 1).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "auth-service");
        assert_eq!(edges[0].to, "shared-types");
    }

    #[test]
    fn test_reverse_dependency_graph_no_dependents() {
        let conn = test_db();
        let edges = reverse_dependency_graph(&conn, "api-gateway", 10).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn test_search_docs() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "docs/auth.md",
                Some("auth-service"),
                "Authentication Guide",
                "How to configure authentication and set up OAuth providers",
                55,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "README.md",
                Option::<String>::None,
                "Project Overview",
                "Welcome to the monorepo project documentation",
                48,
            ),
        )
        .unwrap();

        let results = search_docs(&conn, "authentication", None, 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "docs/auth.md");
        assert_eq!(results[0].package.as_deref(), Some("auth-service"));
        assert_eq!(results[0].title.as_deref(), Some("Authentication Guide"));
    }

    #[test]
    fn test_search_docs_with_package_filter() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "docs/auth.md",
                Some("auth-service"),
                "Auth Setup",
                "How to configure authentication",
                30,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "docs/gateway.md",
                Some("api-gateway"),
                "Gateway Setup",
                "How to configure the gateway authentication proxy",
                50,
            ),
        )
        .unwrap();

        let results = search_docs(&conn, "configure", Some("auth-service"), 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "docs/auth.md");
    }

    #[test]
    fn test_search_docs_empty_query() {
        let conn = test_db();
        let results = search_docs(&conn, "", None, 20).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_docs_special_characters() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "docs/oauth.md",
                Some("auth-service"),
                "OAuth Setup",
                r#"Configure "OAuth" providers with client_id and client_secret"#,
                60,
            ),
        )
        .unwrap();

        // Query with double quotes should not cause FTS5 syntax error
        let results = search_docs(&conn, r#"configure "OAuth""#, None, 20).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_docs_limit_clamping() {
        let conn = test_db();
        // Insert more docs than the limit cap
        for i in 0..5 {
            conn.execute(
                "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
                (format!("docs/guide{i}.md"), Some("auth-service"), format!("Guide {i}"), "How to configure authentication setup", 35),
            ).unwrap();
        }

        let results = search_docs(&conn, "configure", None, 2).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_docs_null_package() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "README.md",
                Option::<String>::None,
                "Project Overview",
                "Welcome to the project documentation",
                36,
            ),
        )
        .unwrap();

        let results = search_docs(&conn, "project", None, 20).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].package.is_none());
    }
}

#[cfg(test)]
mod refs_tests {
    use super::*;
    use crate::db::open_or_create;
    use crate::symbols::{ReferenceInfo, ReferenceKind};
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Seed the `files` table with the given paths and return a file_path →
    /// file_id map — needed because symbol_refs.file_id has a FK to files(id).
    fn seed_files(conn: &Connection, paths: &[&str]) -> HashMap<String, i64> {
        let mut map = HashMap::new();
        for p in paths {
            conn.execute(
                "INSERT INTO files (path, package, extension, size_bytes) VALUES (?1, NULL, '', 0)",
                [p],
            )
            .unwrap();
            let id: i64 = conn
                .query_row("SELECT id FROM files WHERE path = ?1", [p], |r| r.get(0))
                .unwrap();
            map.insert(p.to_string(), id);
        }
        map
    }

    #[test]
    fn test_batch_insert_and_delete_by_file() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("t.db");
        let conn = open_or_create(&db_path).unwrap();
        let mut file_ids = seed_files(&conn, &["a.rs", "b.rs"]);

        let refs = vec![
            ReferenceInfo {
                name: "foo".into(),
                kind: ReferenceKind::Call,
                file_path: Arc::from("a.rs"),
                line: 10,
                enclosing_symbol: Some("bar".into()),
            },
            ReferenceInfo {
                name: "Baz".into(),
                kind: ReferenceKind::Type,
                file_path: Arc::from("a.rs"),
                line: 12,
                enclosing_symbol: None,
            },
            ReferenceInfo {
                name: "quux".into(),
                kind: ReferenceKind::Call,
                file_path: Arc::from("b.rs"),
                line: 3,
                enclosing_symbol: None,
            },
        ];
        batch_insert_references(&conn, Some("mypkg"), &refs, &mut file_ids).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);

        delete_references_for_file(&conn, "a.rs").unwrap();

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, 1, "only b.rs ref should remain");
    }

    #[test]
    fn test_delete_references_for_file_unknown_path_is_noop() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("t_unknown.db");
        let conn = open_or_create(&db_path).unwrap();
        let mut file_ids = seed_files(&conn, &["known.rs"]);

        let refs = vec![ReferenceInfo {
            name: "foo".into(),
            kind: ReferenceKind::Call,
            file_path: Arc::from("known.rs"),
            line: 10,
            enclosing_symbol: Some("main".into()),
        }];
        batch_insert_references(&conn, Some("pkg"), &refs, &mut file_ids).unwrap();

        delete_references_for_file(&conn, "missing.rs").unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "deleting refs for an unknown path should be a no-op"
        );
    }

    #[test]
    fn test_delete_references_for_package() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("t2.db");
        let conn = open_or_create(&db_path).unwrap();
        let mut file_ids = seed_files(&conn, &["x.rs", "y.rs"]);

        let refs_p1 = vec![ReferenceInfo {
            name: "foo".into(),
            kind: ReferenceKind::Call,
            file_path: Arc::from("x.rs"),
            line: 1,
            enclosing_symbol: None,
        }];
        let refs_p2 = vec![ReferenceInfo {
            name: "bar".into(),
            kind: ReferenceKind::Call,
            file_path: Arc::from("y.rs"),
            line: 1,
            enclosing_symbol: None,
        }];
        batch_insert_references(&conn, Some("pkg1"), &refs_p1, &mut file_ids).unwrap();
        batch_insert_references(&conn, Some("pkg2"), &refs_p2, &mut file_ids).unwrap();

        delete_references_for_package(&conn, "pkg1").unwrap();

        let remaining_pkg: String = conn
            .query_row("SELECT package FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining_pkg, "pkg2");
    }

    #[test]
    fn test_batch_insert_references_lazy_file_id_lookup() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("lazy.db");
        let conn = open_or_create(&db_path).unwrap();
        seed_files(&conn, &["src/main.rs"]);

        let mut file_ids = build_file_id_map(&conn).unwrap();
        assert!(
            file_ids.is_empty(),
            "file-id cache starts empty and warms lazily"
        );

        let refs = vec![ReferenceInfo {
            name: "run".into(),
            kind: ReferenceKind::Call,
            file_path: Arc::from("src/main.rs"),
            line: 8,
            enclosing_symbol: Some("main".into()),
        }];
        batch_insert_references(&conn, Some("app"), &refs, &mut file_ids).unwrap();

        assert!(
            file_ids.contains_key("src/main.rs"),
            "cache should warm after first lookup"
        );
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_query_symbol_references_filters() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("r.db");
        let conn = open_or_create(&db_path).unwrap();
        let ids = seed_files(&conn, &["a.rs", "b.rs"]);
        let a = ids["a.rs"];
        let b = ids["b.rs"];

        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                 VALUES ('foo', 'call', {a}, 10, 'pkg1', 'bar'), \
                        ('foo', 'type', {a}, 20, 'pkg1', 'bar'), \
                        ('foo', 'call', {b}, 5, 'pkg2', 'quux'), \
                        ('other', 'call', {a}, 30, 'pkg1', NULL)"
            ),
            [],
        )
        .unwrap();

        let all = query_symbol_references(&conn, "foo", None, None, 100).unwrap();
        assert_eq!(all.len(), 3);

        let calls = query_symbol_references(&conn, "foo", Some("call"), None, 100).unwrap();
        assert_eq!(calls.len(), 2);

        let p1 = query_symbol_references(&conn, "foo", None, Some("pkg1"), 100).unwrap();
        assert_eq!(p1.len(), 2);

        let p1_calls =
            query_symbol_references(&conn, "foo", Some("call"), Some("pkg1"), 100).unwrap();
        assert_eq!(p1_calls.len(), 1);
    }

    #[test]
    fn test_query_symbol_references_returns_rows_without_symbol_definition() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("r_unknown_symbol.db");
        let conn = open_or_create(&db_path).unwrap();
        let ids = seed_files(&conn, &["ext.rs"]);
        let ext = ids["ext.rs"];

        // No row in `symbols` for "ExternalThing" on purpose. `symbol_references`
        // is name-based over symbol_refs and should still return this row.
        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                 VALUES ('ExternalThing', 'call', {ext}, 7, 'pkg-ext', 'main')"
            ),
            [],
        )
        .unwrap();

        let rows =
            query_symbol_references(&conn, "ExternalThing", Some("call"), Some("pkg-ext"), 10)
                .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "ExternalThing");
        assert_eq!(rows[0].kind, "call");
        assert_eq!(rows[0].file_path, "ext.rs");
        assert_eq!(rows[0].line, 7);
        assert_eq!(rows[0].package.as_deref(), Some("pkg-ext"));
        assert_eq!(rows[0].enclosing_symbol.as_deref(), Some("main"));
    }

    #[test]
    fn test_query_symbol_references_unicode_round_trip_nfc_and_nfd() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("r_unicode.db");
        let conn = open_or_create(&db_path).unwrap();
        let ids = seed_files(&conn, &["unicode.rs"]);
        let file_id = ids["unicode.rs"];

        let nfc = "Café";
        let nfd = "Cafe\u{301}";
        assert_ne!(
            nfc, nfd,
            "test requires distinct byte sequences for NFC/NFD forms"
        );

        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                 VALUES ('Config', 'type', {file_id}, 10, 'pkg-u', NULL), \
                        (?1,      'type', {file_id}, 11, 'pkg-u', NULL), \
                        (?2,      'type', {file_id}, 12, 'pkg-u', NULL), \
                        ('Ω',     'type', {file_id}, 13, 'pkg-u', NULL)"
            ),
            (nfc, nfd),
        )
        .unwrap();

        let config =
            query_symbol_references(&conn, "Config", Some("type"), Some("pkg-u"), 10).unwrap();
        assert_eq!(config.len(), 1);
        assert_eq!(config[0].name, "Config");
        assert_eq!(config[0].line, 10);

        let nfc_rows =
            query_symbol_references(&conn, nfc, Some("type"), Some("pkg-u"), 10).unwrap();
        assert_eq!(nfc_rows.len(), 1);
        assert_eq!(nfc_rows[0].name, nfc);
        assert_eq!(nfc_rows[0].line, 11);

        let nfd_rows =
            query_symbol_references(&conn, nfd, Some("type"), Some("pkg-u"), 10).unwrap();
        assert_eq!(nfd_rows.len(), 1);
        assert_eq!(nfd_rows[0].name, nfd);
        assert_eq!(nfd_rows[0].line, 12);

        let omega = query_symbol_references(&conn, "Ω", Some("type"), Some("pkg-u"), 10).unwrap();
        assert_eq!(omega.len(), 1);
        assert_eq!(omega[0].name, "Ω");
        assert_eq!(omega[0].line, 13);
    }

    #[test]
    fn test_query_symbol_callers_groups_and_counts() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("c.db");
        let conn = open_or_create(&db_path).unwrap();
        let ids = seed_files(&conn, &["a.rs", "b.rs", "c.rs"]);
        let a = ids["a.rs"];
        let b = ids["b.rs"];
        let c = ids["c.rs"];

        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                 VALUES ('foo', 'call', {a}, 10, 'p', 'bar'), \
                        ('foo', 'call', {a}, 11, 'p', 'bar'), \
                        ('foo', 'call', {b}, 5, 'p', 'quux'), \
                        ('foo', 'type', {a}, 20, 'p', 'bar'), \
                        ('foo', 'call', {c}, 1, 'p', NULL), \
                        ('foo', 'call', {c}, 2, 'p', NULL), \
                        ('foo', 'call', {c}, 3, 'p', NULL)"
            ),
            [],
        )
        .unwrap();

        let callers = query_symbol_callers(&conn, "foo", None, 100).unwrap();
        assert_eq!(callers.len(), 2);
        let bar = callers.iter().find(|c| c.caller_name == "bar").unwrap();
        assert_eq!(bar.call_sites, 2);
        assert_eq!(bar.caller_file, "a.rs");
        assert_eq!(bar.caller_line, 10);

        let p_only = query_symbol_callers(&conn, "foo", Some("p"), 100).unwrap();
        assert_eq!(p_only.len(), 2);
        let none = query_symbol_callers(&conn, "foo", Some("other"), 100).unwrap();
        assert!(none.is_empty());

        // Proves `r.enclosing_symbol IS NOT NULL` stays in the SQL.
        // If the filter were removed, the NULL group has the highest call-site
        // count (3) and would sort first; with `LIMIT 1` that top row would be
        // deserialization-dropped and we'd lose the real caller row.
        let top = query_symbol_callers(&conn, "foo", None, 1).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].caller_name, "bar");
        assert_eq!(top[0].call_sites, 2);
    }

    #[test]
    fn test_query_symbol_callees_groups() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cee.db");
        let conn = open_or_create(&db_path).unwrap();
        let ids = seed_files(&conn, &["a.rs"]);
        let a = ids["a.rs"];

        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                 VALUES ('foo', 'call', {a}, 1, 'p', 'handler'), \
                        ('bar', 'call', {a}, 2, 'p', 'handler'), \
                        ('foo', 'call', {a}, 3, 'p', 'handler'), \
                        ('baz', 'call', {a}, 4, 'p', 'other')"
            ),
            [],
        )
        .unwrap();

        let callees = query_symbol_callees(&conn, "handler", None, 100).unwrap();
        assert_eq!(callees.len(), 2);
        let foo = callees.iter().find(|c| c.callee_name == "foo").unwrap();
        assert_eq!(foo.call_sites, 2);

        let p_only = query_symbol_callees(&conn, "handler", Some("p"), 100).unwrap();
        assert_eq!(p_only.len(), 2);
        let none = query_symbol_callees(&conn, "handler", Some("other"), 100).unwrap();
        assert!(none.is_empty());
    }

    /// Seed a package row for dependency graph tests. Each package needs a
    /// row in `packages` for the FK on `dependencies.package` to hold.
    fn seed_package(conn: &Connection, name: &str) {
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES (?1, ?2, 'rust')",
            [name, name],
        )
        .unwrap();
    }

    fn seed_dependency(conn: &Connection, pkg: &str, dep: &str) {
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES (?1, ?2, 'runtime', 1)",
            [pkg, dep],
        )
        .unwrap();
    }

    /// Full-stack test of change_impact: home-package identification from
    /// `symbols`, partitioning refs into direct vs cross-package, and BFS of
    /// the reverse dep graph for transitive impact.
    #[test]
    fn test_change_impact_partitions_and_traverses() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("ci.db");
        let conn = open_or_create(&db_path).unwrap();

        // Packages + dep graph:
        //   api-server → config-core
        //   worker → config-core
        //   integration-tests → api-server
        //   deploy-scripts → worker
        for p in [
            "config-core",
            "api-server",
            "worker",
            "integration-tests",
            "deploy-scripts",
        ] {
            seed_package(&conn, p);
        }
        seed_dependency(&conn, "api-server", "config-core");
        seed_dependency(&conn, "worker", "config-core");
        seed_dependency(&conn, "integration-tests", "api-server");
        seed_dependency(&conn, "deploy-scripts", "worker");

        // `parseConfig` defined in config-core
        conn.execute(
            "INSERT INTO symbols (package, name, kind, file_path, line) \
             VALUES ('config-core', 'parseConfig', 'function', 'config-core/src/lib.rs', 5)",
            [],
        )
        .unwrap();

        let ids = seed_files(
            &conn,
            &[
                "config-core/src/loader.rs",
                "config-core/src/validate.rs",
                "api-server/src/main.rs",
                "worker/src/init.rs",
            ],
        );
        let loader = ids["config-core/src/loader.rs"];
        let validate = ids["config-core/src/validate.rs"];
        let api_main = ids["api-server/src/main.rs"];
        let worker_init = ids["worker/src/init.rs"];

        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) VALUES \
                 ('parseConfig', 'call', {loader}, 42, 'config-core', 'load'), \
                 ('parseConfig', 'call', {validate}, 18, 'config-core', 'validate'), \
                 ('parseConfig', 'call', {api_main}, 7, 'api-server', 'main'), \
                 ('parseConfig', 'call', {worker_init}, 23, 'worker', 'init')"
            ),
            [],
        )
        .unwrap();

        let impact = change_impact(&conn, "parseConfig", None, 2, 100).unwrap();

        assert_eq!(impact.home_package.as_deref(), Some("config-core"));
        assert_eq!(impact.direct_impact.len(), 2);
        assert_eq!(impact.cross_package_impact.len(), 2);
        assert_eq!(
            impact.summary.affected_packages,
            vec!["api-server", "worker"]
        );

        // Transitive: integration-tests → api-server, deploy-scripts → worker.
        // Neither api-server nor worker themselves appear (they're in
        // affected_packages and excluded from transitive).
        let trans_pkgs: HashSet<&str> = impact
            .transitive_impact
            .iter()
            .map(|t| t.package.as_str())
            .collect();
        assert_eq!(
            trans_pkgs,
            HashSet::from(["integration-tests", "deploy-scripts"])
        );
        assert_eq!(impact.summary.transitive_package_count, 2);
        for t in &impact.transitive_impact {
            assert_eq!(t.distance, 1);
        }
    }

    /// When the caller supplies a `package` hint, it overrides lookup from
    /// the `symbols` table. This disambiguates same-name symbols defined
    /// in multiple packages.
    #[test]
    fn test_change_impact_package_hint_overrides() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("ci2.db");
        let conn = open_or_create(&db_path).unwrap();

        for p in ["pkgA", "pkgB"] {
            seed_package(&conn, p);
        }
        // Symbol defined in both packages — lookup would return pkgA (alphabetical).
        conn.execute(
            "INSERT INTO symbols (package, name, kind, file_path, line) \
             VALUES ('pkgA', 'doThing', 'function', 'a.rs', 1), \
                    ('pkgB', 'doThing', 'function', 'b.rs', 1)",
            [],
        )
        .unwrap();

        let ids = seed_files(&conn, &["a.rs", "b.rs"]);
        let a = ids["a.rs"];
        let b = ids["b.rs"];
        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) VALUES \
                 ('doThing', 'call', {a}, 5, 'pkgA', NULL), \
                 ('doThing', 'call', {b}, 5, 'pkgB', NULL)"
            ),
            [],
        )
        .unwrap();

        // Hint pkgB — ref in pkgB is direct, ref in pkgA is cross-package.
        let impact = change_impact(&conn, "doThing", Some("pkgB"), 1, 100).unwrap();
        assert_eq!(impact.home_package.as_deref(), Some("pkgB"));
        assert_eq!(impact.direct_impact.len(), 1);
        assert_eq!(impact.direct_impact[0].package.as_deref(), Some("pkgB"));
        assert_eq!(impact.cross_package_impact.len(), 1);
        assert_eq!(
            impact.cross_package_impact[0].package.as_deref(),
            Some("pkgA")
        );
    }

    /// When the symbol has no definition in `symbols` and no hint is given,
    /// home_package is None and every ref becomes cross-package. The tool
    /// still works — impact analysis is useful for undefined/external symbols
    /// (imports of third-party names, macros, etc.).
    #[test]
    fn test_change_impact_no_home_package() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("ci3.db");
        let conn = open_or_create(&db_path).unwrap();

        for p in ["pkg1", "pkg2"] {
            seed_package(&conn, p);
        }
        let ids = seed_files(&conn, &["a.rs", "b.rs"]);
        let a = ids["a.rs"];
        let b = ids["b.rs"];
        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) VALUES \
                 ('External', 'type', {a}, 1, 'pkg1', NULL), \
                 ('External', 'type', {b}, 1, 'pkg2', NULL)"
            ),
            [],
        )
        .unwrap();

        let impact = change_impact(&conn, "External", None, 0, 100).unwrap();
        assert!(impact.home_package.is_none());
        assert_eq!(impact.direct_impact.len(), 0);
        assert_eq!(impact.cross_package_impact.len(), 2);
        assert_eq!(impact.summary.affected_packages, vec!["pkg1", "pkg2"]);
        assert_eq!(impact.transitive_impact.len(), 0);
    }

    /// Depth=0 disables transitive traversal entirely, even when there are
    /// reverse-dep edges to follow.
    #[test]
    fn test_change_impact_transitive_depth_zero() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("ci4.db");
        let conn = open_or_create(&db_path).unwrap();

        for p in ["core", "consumer", "grandchild"] {
            seed_package(&conn, p);
        }
        seed_dependency(&conn, "consumer", "core");
        seed_dependency(&conn, "grandchild", "consumer");

        conn.execute(
            "INSERT INTO symbols (package, name, kind, file_path, line) \
             VALUES ('core', 'foo', 'function', 'core.rs', 1)",
            [],
        )
        .unwrap();
        let ids = seed_files(&conn, &["consumer.rs"]);
        let c = ids["consumer.rs"];
        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                 VALUES ('foo', 'call', {c}, 1, 'consumer', NULL)"
            ),
            [],
        )
        .unwrap();

        let impact = change_impact(&conn, "foo", None, 0, 100).unwrap();
        assert_eq!(impact.cross_package_impact.len(), 1);
        assert_eq!(impact.transitive_impact.len(), 0);

        // Depth=2 should find grandchild (consumer → grandchild).
        let deep = change_impact(&conn, "foo", None, 2, 100).unwrap();
        assert_eq!(deep.transitive_impact.len(), 1);
        assert_eq!(deep.transitive_impact[0].package, "grandchild");
        assert_eq!(deep.transitive_impact[0].distance, 1);
        assert_eq!(deep.transitive_impact[0].via, "consumer");
    }

    /// Regression test: when one bucket would dominate the sorted fetch,
    /// both buckets must still populate correctly and affected_packages
    /// must reflect the full cross-package set. Previously the 2*limit
    /// prefetch could starve a bucket entirely.
    #[test]
    fn test_change_impact_does_not_starve_bucket() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("ci_starve.db");
        let conn = open_or_create(&db_path).unwrap();

        seed_package(&conn, "home");
        seed_package(&conn, "consumer_z");

        conn.execute(
            "INSERT INTO symbols (package, name, kind, file_path, line) \
             VALUES ('home', 'foo', 'function', 'home/src.rs', 1)",
            [],
        )
        .unwrap();

        // Many home refs in alphabetically-early paths (home/a..home/y),
        // few cross-package refs at the end (zzz/). Sorted by file_path,
        // home refs come first — with a low prefetch, the cross ref would
        // never be seen.
        let mut home_paths: Vec<String> = (0..20).map(|i| format!("home/a{i:02}.rs")).collect();
        home_paths.push("zzz/x.rs".into());
        let path_refs: Vec<&str> = home_paths.iter().map(|s| s.as_str()).collect();
        let ids = seed_files(&conn, &path_refs);

        let mut values = Vec::new();
        for p in &home_paths[..20] {
            let id = ids[p];
            values.push(format!("('foo', 'call', {id}, 1, 'home', NULL)"));
        }
        let z_id = ids["zzz/x.rs"];
        values.push(format!("('foo', 'call', {z_id}, 1, 'consumer_z', NULL)"));

        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) VALUES {}",
                values.join(", ")
            ),
            [],
        )
        .unwrap();

        // Request a small limit — the bug would cause cross_package_impact
        // to be empty because the 2*5=10 prefetch would only see home refs.
        let impact = change_impact(&conn, "foo", None, 1, 5).unwrap();

        // Pre-truncation counts must reflect the true partition.
        assert_eq!(impact.summary.direct_count, 20);
        assert_eq!(impact.summary.cross_package_count, 1);
        assert_eq!(impact.summary.affected_packages, vec!["consumer_z"]);

        // Returned buckets are truncated to per_bucket_limit.
        assert_eq!(impact.direct_impact.len(), 5);
        assert_eq!(impact.cross_package_impact.len(), 1);
    }

    /// Regression test: external dependency edges (is_internal = 0) must
    /// not contribute transitive impact. This matches reverse_dependency_graph.
    #[test]
    fn test_change_impact_skips_external_dep_edges() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("ci_ext.db");
        let conn = open_or_create(&db_path).unwrap();

        for p in ["home", "consumer", "external_collider"] {
            seed_package(&conn, p);
        }
        // `external_collider` declares an external dep named `consumer` —
        // a third-party library that happens to share a name with our
        // internal package. Must NOT show up as transitive impact.
        conn.execute(
            "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES ('external_collider','consumer','runtime',0)",
            [],
        )
        .unwrap();
        // Also a legitimate internal edge for contrast.
        seed_package(&conn, "legit_dep");
        seed_dependency(&conn, "legit_dep", "consumer");

        conn.execute(
            "INSERT INTO symbols (package, name, kind, file_path, line) \
             VALUES ('home', 'foo', 'function', 'home.rs', 1)",
            [],
        )
        .unwrap();
        let ids = seed_files(&conn, &["consumer/a.rs"]);
        let c = ids["consumer/a.rs"];
        conn.execute(
            &format!(
                "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
                 VALUES ('foo', 'call', {c}, 1, 'consumer', NULL)"
            ),
            [],
        )
        .unwrap();

        let impact = change_impact(&conn, "foo", None, 3, 100).unwrap();
        let trans_pkgs: HashSet<&str> = impact
            .transitive_impact
            .iter()
            .map(|t| t.package.as_str())
            .collect();
        assert_eq!(trans_pkgs, HashSet::from(["legit_dep"]));
        assert!(!trans_pkgs.contains("external_collider"));
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use crate::db::open_or_create;
    use tempfile::tempdir;

    #[test]
    fn test_insert_and_query_schema_consumers() {
        let dir = tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("b.db")).unwrap();

        let edges = vec![
            BoundaryEdge {
                source_path: "proto/user.proto".into(),
                generated_path: "gen/user.pb.go".into(),
                source_package: Some("proto-pkg".into()),
                generated_package: Some("go-pkg".into()),
                kind: "proto".into(),
            },
            BoundaryEdge {
                source_path: "proto/user.proto".into(),
                generated_path: "gen/user_pb2.py".into(),
                source_package: Some("proto-pkg".into()),
                generated_package: Some("py-pkg".into()),
                kind: "proto".into(),
            },
        ];
        batch_insert_boundary_edges(&conn, &edges).unwrap();

        let consumers = query_schema_consumers(&conn, "proto/user.proto").unwrap();
        assert_eq!(consumers.len(), 2);

        let from = query_generated_from(&conn, "gen/user.pb.go").unwrap();
        assert_eq!(from.len(), 1);
        assert_eq!(from[0].source_path, "proto/user.proto");
    }

    #[test]
    fn test_clear_boundary_edges() {
        let dir = tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("b2.db")).unwrap();

        let edges = vec![BoundaryEdge {
            source_path: "a.proto".into(),
            generated_path: "a.pb.go".into(),
            source_package: None,
            generated_package: None,
            kind: "proto".into(),
        }];
        batch_insert_boundary_edges(&conn, &edges).unwrap();
        clear_boundary_edges(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM boundary_edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
