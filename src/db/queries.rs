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

/// FTS5 search across package name, description, and path. Returns up to 20 results.
pub fn search_packages(conn: &Connection, query: &str) -> Result<Vec<PackageRow>> {
    let mut stmt = conn.prepare(
        "SELECT p.name, p.path, p.kind, p.version, p.description, p.metadata
         FROM packages_fts f
         JOIN packages p ON p.name = f.name
         WHERE packages_fts MATCH ?1
         LIMIT 20",
    )?;
    let rows = stmt.query_map([query], |row| {
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

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        let mut stmt = conn.prepare(sql)?;
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
}
