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
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match (package_filter, kind_filter) {
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
            vec![Box::new(fts_query) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string()), Box::new(limit)],
        )},
        (Some(pkg), None) => (
            "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1 AND s.package = ?2
             ORDER BY rank
             LIMIT ?3",
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string()), Box::new(limit)],
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
            vec![Box::new(fts_query) as Box<dyn rusqlite::types::ToSql>, Box::new(limit)],
        )},
        (None, None) => (
            "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(limit)],
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

/// Fetch symbols by their id (primary key). Used by hybrid search to look up
/// vector search results. Returns results in unspecified order.
#[allow(dead_code)]
pub fn get_symbols_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<(i64, SymbolRow)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT id, name, kind, signature, package, file_path, line,
                visibility, parent_symbol, return_type, parameters
         FROM symbols
         WHERE id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<Box<dyn rusqlite::types::ToSql>> =
        ids.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            SymbolRow {
                name: row.get(1)?,
                kind: row.get(2)?,
                signature: row.get(3)?,
                package: row.get(4)?,
                file_path: row.get(5)?,
                line: row.get(6)?,
                visibility: row.get(7)?,
                parent_symbol: row.get(8)?,
                return_type: row.get(9)?,
                parameters: row.get(10)?,
            },
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Merge two ranked result lists using Reciprocal Rank Fusion (RRF) with k=60.
/// Results appearing in both lists score higher. Returns merged list sorted by
/// RRF score descending, truncated to `limit`.
#[allow(dead_code)]
pub fn rrf_merge(
    fts_results: &[SymbolRow],
    vec_results: &[SymbolRow],
    limit: usize,
) -> Vec<SymbolRow> {
    use std::collections::HashMap;

    type SymKey = (String, String, String, i64);
    fn sym_key(s: &SymbolRow) -> SymKey {
        (s.name.clone(), s.package.clone(), s.file_path.clone(), s.line)
    }

    let k = 60.0_f64;
    let mut scores: HashMap<SymKey, f64> = HashMap::new();
    let mut symbols: HashMap<SymKey, SymbolRow> = HashMap::new();

    for (rank, sym) in fts_results.iter().enumerate() {
        let key = sym_key(sym);
        *scores.entry(key.clone()).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
        symbols.entry(key).or_insert_with(|| sym.clone());
    }

    for (rank, sym) in vec_results.iter().enumerate() {
        let key = sym_key(sym);
        *scores.entry(key.clone()).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
        symbols.entry(key).or_insert_with(|| sym.clone());
    }

    let mut scored: Vec<(SymKey, f64)> = scores.into_iter().collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    scored
        .into_iter()
        .take(limit)
        .filter_map(|(key, _)| symbols.remove(&key))
        .collect()
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

    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match (package_filter, extension_filter) {
        (Some(pkg), Some(ext)) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.package = ?2 AND f.extension = ?3
             ORDER BY rank
             LIMIT ?4",
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string()), Box::new(ext.to_string()), Box::new(limit)],
        ),
        (Some(pkg), None) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.package = ?2
             ORDER BY rank
             LIMIT ?3",
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string()), Box::new(limit)],
        ),
        (None, Some(ext)) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.extension = ?2
             ORDER BY rank
             LIMIT ?3",
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(ext.to_string()), Box::new(limit)],
        ),
        (None, None) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(limit)],
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

    let sql = "SELECT package, dep_kind FROM dependencies WHERE dependency = ?1 AND is_internal = 1";

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
        ).unwrap();
        conn.execute(
            "INSERT INTO shire_meta (key, value) VALUES (?1, ?2)",
            ("git_commit", "abc123"),
        ).unwrap();
        conn.execute(
            "INSERT INTO shire_meta (key, value) VALUES (?1, ?2)",
            ("package_count", "3"),
        ).unwrap();
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
        let results = search_symbols(&conn, "validate", Some("auth-service"), Some("method"), 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "validate");

        // Wrong package
        let results = search_symbols(&conn, "validate", Some("nonexistent"), Some("method"), 20).unwrap();
        assert!(results.is_empty());

        // Wrong kind
        let results = search_symbols(&conn, "validate", Some("auth-service"), Some("class"), 20).unwrap();
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
            ("services/auth/src/auth.ts", Some("auth-service"), "ts", 1024i64),
            ("services/auth/src/middleware.ts", Some("auth-service"), "ts", 512),
            ("services/auth/package.json", Some("auth-service"), "json", 256),
            ("packages/shared-types/src/types.ts", Some("shared-types"), "ts", 2048),
            ("services/gateway/main.go", Some("api-gateway"), "go", 4096),
            ("services/gateway/handler.go", Some("api-gateway"), "go", 3072),
            ("scripts/deploy.sh", None, "sh", 128),
            ("README.md", None, "md", 64),
        ];
        for (path, package, ext, size) in &files {
            conn.execute(
                "INSERT INTO files (path, package, extension, size_bytes) VALUES (?1, ?2, ?3, ?4)",
                (path, package, ext, size),
            ).unwrap();
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
        assert!(results.iter().all(|f| f.package.as_deref() == Some("auth-service")));
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
        assert!(results.iter().all(|f| f.package.as_deref() == Some("auth-service") && f.extension == "ts"));
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

    fn make_symbol(name: &str, package: &str, line: i64) -> SymbolRow {
        SymbolRow {
            name: name.into(),
            kind: "function".into(),
            signature: None,
            package: package.into(),
            file_path: format!("src/{name}.rs"),
            line,
            visibility: "public".into(),
            parent_symbol: None,
            return_type: None,
            parameters: None,
        }
    }

    #[test]
    fn test_rrf_merge_overlapping_results() {
        let fts = vec![
            make_symbol("alpha", "pkg-a", 10),   // rank 0 in FTS
            make_symbol("beta", "pkg-a", 20),    // rank 1 in FTS
            make_symbol("gamma", "pkg-b", 30),   // rank 2 in FTS
        ];
        let vec = vec![
            make_symbol("beta", "pkg-a", 20),    // rank 0 in vec (also in FTS)
            make_symbol("delta", "pkg-b", 40),   // rank 1 in vec
            make_symbol("alpha", "pkg-a", 10),   // rank 2 in vec (also in FTS)
        ];

        let merged = rrf_merge(&fts, &vec, 50);
        assert_eq!(merged.len(), 4);

        // beta appears in both at high ranks → highest RRF score
        // alpha appears in both → second highest
        // Overlapping results should rank higher than single-source
        let names: Vec<&str> = merged.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names[0], "beta");  // rank 0 in vec + rank 1 in FTS
        assert_eq!(names[1], "alpha"); // rank 0 in FTS + rank 2 in vec
        // gamma and delta are single-source, order depends on rank
    }

    #[test]
    fn test_rrf_merge_disjoint_results() {
        let fts = vec![
            make_symbol("alpha", "pkg-a", 10),
            make_symbol("beta", "pkg-a", 20),
        ];
        let vec = vec![
            make_symbol("gamma", "pkg-b", 30),
            make_symbol("delta", "pkg-b", 40),
        ];

        let merged = rrf_merge(&fts, &vec, 50);
        assert_eq!(merged.len(), 4);

        // All single-source, rank 0 items from each list tie at 1/(60+1) ≈ 0.01639
        let names: Vec<&str> = merged.iter().map(|s| s.name.as_str()).collect();
        // Both rank-0 items tie, both rank-1 items tie
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));
        assert!(names.contains(&"delta"));
    }

    #[test]
    fn test_rrf_merge_respects_limit() {
        let fts = vec![
            make_symbol("a", "pkg", 1),
            make_symbol("b", "pkg", 2),
            make_symbol("c", "pkg", 3),
        ];
        let vec = vec![
            make_symbol("d", "pkg", 4),
            make_symbol("e", "pkg", 5),
        ];

        let merged = rrf_merge(&fts, &vec, 3);
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn test_rrf_merge_empty_inputs() {
        let fts: Vec<SymbolRow> = vec![];
        let vec = vec![make_symbol("alpha", "pkg", 10)];

        let merged = rrf_merge(&fts, &vec, 50);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "alpha");

        let merged = rrf_merge(&vec, &fts, 50);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "alpha");
    }

    #[test]
    fn test_rrf_merge_both_empty() {
        let fts: Vec<SymbolRow> = vec![];
        let vec: Vec<SymbolRow> = vec![];
        let merged = rrf_merge(&fts, &vec, 50);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_get_symbols_by_ids_returns_matching() {
        let conn = test_db_with_symbols();
        let all_ids: Vec<i64> = conn
            .prepare("SELECT id FROM symbols")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!all_ids.is_empty());

        let results = get_symbols_by_ids(&conn, &all_ids).unwrap();
        assert_eq!(results.len(), all_ids.len());

        for (id, sym) in &results {
            assert!(all_ids.contains(id));
            assert!(!sym.name.is_empty());
            assert!(!sym.package.is_empty());
        }
    }

    #[test]
    fn test_get_symbols_by_ids_empty_input() {
        let conn = test_db_with_symbols();
        let results = get_symbols_by_ids(&conn, &[]).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_symbols_by_ids_nonexistent() {
        let conn = test_db_with_symbols();
        let results = get_symbols_by_ids(&conn, &[99999, 99998]).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_symbols_by_ids_field_mapping() {
        let conn = test_db_with_symbols();
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM symbols WHERE name = 'validate'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(ids.len(), 1);

        let results = get_symbols_by_ids(&conn, &ids).unwrap();
        assert_eq!(results.len(), 1);
        let (_, sym) = &results[0];
        assert_eq!(sym.name, "validate");
        assert_eq!(sym.kind, "method");
        assert_eq!(sym.package, "auth-service");
        assert_eq!(sym.parent_symbol.as_deref(), Some("AuthService"));
        assert_eq!(sym.return_type.as_deref(), Some("Promise<boolean>"));
    }

    #[test]
    fn test_search_docs() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("docs/auth.md", Some("auth-service"), "Authentication Guide", "How to configure authentication and set up OAuth providers", 55),
        ).unwrap();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("README.md", Option::<String>::None, "Project Overview", "Welcome to the monorepo project documentation", 48),
        ).unwrap();

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
            ("docs/auth.md", Some("auth-service"), "Auth Setup", "How to configure authentication", 30),
        ).unwrap();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("docs/gateway.md", Some("api-gateway"), "Gateway Setup", "How to configure the gateway authentication proxy", 50),
        ).unwrap();

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
}
