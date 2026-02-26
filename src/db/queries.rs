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
}

#[derive(Debug, Serialize)]
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

/// FTS5 search across symbol names and signatures. Returns up to 50 results.
pub fn search_symbols(
    conn: &Connection,
    query: &str,
    package_filter: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<Vec<SymbolRow>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let sanitized = format!("\"{}\"", query.replace('"', "\"\""));

    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match (package_filter, kind_filter) {
        (Some(pkg), Some(kind)) => (
            "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1 AND s.package = ?2 AND s.kind = ?3
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string()), Box::new(kind.to_string())],
        ),
        (Some(pkg), None) => (
            "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1 AND s.package = ?2
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string())],
        ),
        (None, Some(kind)) => (
            "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1 AND s.kind = ?2
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(kind.to_string())],
        ),
        (None, None) => (
            "SELECT s.name, s.kind, s.signature, s.package, s.file_path, s.line,
                    s.visibility, s.parent_symbol, s.return_type, s.parameters
             FROM symbols_fts f
             JOIN symbols s ON s.rowid = f.rowid
             WHERE symbols_fts MATCH ?1
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
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
    let mut stmt = conn.prepare(sql)?;
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

/// Look up symbols by exact name, optionally scoped to a package.
pub fn get_symbol(
    conn: &Connection,
    name: &str,
    package_filter: Option<&str>,
) -> Result<Vec<SymbolRow>> {
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match package_filter {
        Some(pkg) => (
            "SELECT name, kind, signature, package, file_path, line,
                    visibility, parent_symbol, return_type, parameters
             FROM symbols
             WHERE name = ?1 AND package = ?2",
            vec![Box::new(name.to_string()), Box::new(pkg.to_string())],
        ),
        None => (
            "SELECT name, kind, signature, package, file_path, line,
                    visibility, parent_symbol, return_type, parameters
             FROM symbols
             WHERE name = ?1",
            vec![Box::new(name.to_string())],
        ),
    };
    let mut stmt = conn.prepare(sql)?;
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

/// FTS5 search across file paths. Returns up to 50 results.
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

    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match (package_filter, extension_filter) {
        (Some(pkg), Some(ext)) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.package = ?2 AND f.extension = ?3
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string()), Box::new(ext.to_string())],
        ),
        (Some(pkg), None) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.package = ?2
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(pkg.to_string())],
        ),
        (None, Some(ext)) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1 AND f.extension = ?2
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>, Box::new(ext.to_string())],
        ),
        (None, None) => (
            "SELECT f.path, f.package, f.extension, f.size_bytes
             FROM files_fts fts
             JOIN files f ON f.rowid = fts.rowid
             WHERE files_fts MATCH ?1
             LIMIT 50".to_string(),
            vec![Box::new(sanitized) as Box<dyn rusqlite::types::ToSql>],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
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
    let mut stmt = conn.prepare(sql)?;
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

/// FTS5 search across package name, description, and path. Returns up to 20 results.
pub fn search_packages(conn: &Connection, query: &str) -> Result<Vec<PackageRow>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Sanitize for FTS5: wrap in double quotes, escape internal quotes
    let sanitized = format!("\"{}\"", query.replace('"', "\"\""));
    let mut stmt = conn.prepare(
        "SELECT p.name, p.path, p.kind, p.version, p.description, p.metadata
         FROM packages_fts f
         JOIN packages p ON p.name = f.name
         WHERE packages_fts MATCH ?1
         LIMIT 20",
    )?;
    let rows = stmt.query_map([&sanitized], |row| {
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
pub fn get_package(conn: &Connection, name: &str) -> Result<Option<PackageRow>> {
    let mut stmt = conn.prepare(
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
    let mut stmt = conn.prepare(sql)?;
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
    let mut stmt = conn.prepare(
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
pub fn dependency_graph(
    conn: &Connection,
    root: &str,
    max_depth: u32,
    internal_only: bool,
) -> Result<Vec<GraphEdge>> {
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

    let mut stmt = conn.prepare(sql)?;

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
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
    let mut stmt = conn.prepare(sql)?;
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
        let mut stmt = conn.prepare("SELECT value FROM shire_meta WHERE key = ?1")?;
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
    })
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
        let results = search_packages(&conn, "auth").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "auth-service");
    }

    #[test]
    fn test_search_packages_finds_by_description() {
        let conn = test_db();
        let results = search_packages(&conn, "TypeScript").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "shared-types");
    }

    #[test]
    fn test_search_packages_no_match() {
        let conn = test_db();
        let results = search_packages(&conn, "nonexistent").unwrap();
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
        let results = search_symbols(&conn, "AuthService", None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "AuthService");
        assert_eq!(results[0].package, "auth-service");
    }

    #[test]
    fn test_search_symbols_by_signature() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "token", None, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "validate");
    }

    #[test]
    fn test_search_symbols_filter_by_package() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "interface", Some("shared-types"), None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "UserConfig");

        let results = search_symbols(&conn, "interface", Some("auth-service"), None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_symbols_filter_by_kind() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "AuthService", None, Some("class")).unwrap();
        assert_eq!(results.len(), 1);

        let results = search_symbols(&conn, "AuthService", None, Some("function")).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_symbols_empty_query() {
        let conn = test_db_with_symbols();
        let results = search_symbols(&conn, "", None, None).unwrap();
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

    #[test]
    fn test_get_symbol() {
        let conn = test_db_with_symbols();
        let results = get_symbol(&conn, "AuthService", None).unwrap();
        assert_eq!(results.len(), 1);

        let results = get_symbol(&conn, "AuthService", Some("auth-service")).unwrap();
        assert_eq!(results.len(), 1);

        let results = get_symbol(&conn, "AuthService", Some("shared-types")).unwrap();
        assert!(results.is_empty());

        let results = get_symbol(&conn, "nonexistent", None).unwrap();
        assert!(results.is_empty());
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
}
