pub mod queries;

use anyhow::Result;
use rusqlite::Connection;

pub fn open_or_create(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;
    Ok(conn)
}

pub fn open_readonly(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(conn)
}

fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS packages (
            name        TEXT PRIMARY KEY,
            path        TEXT NOT NULL UNIQUE,
            kind        TEXT NOT NULL,
            version     TEXT,
            description TEXT,
            metadata    TEXT
        );

        CREATE TABLE IF NOT EXISTS dependencies (
            package     TEXT NOT NULL REFERENCES packages(name),
            dependency  TEXT NOT NULL,
            dep_kind    TEXT NOT NULL DEFAULT 'runtime',
            version_req TEXT,
            is_internal INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (package, dependency, dep_kind)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS packages_fts USING fts5(
            name, description, path,
            content='packages',
            content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS packages_ai AFTER INSERT ON packages BEGIN
            INSERT INTO packages_fts(rowid, name, description, path)
            VALUES (new.rowid, new.name, new.description, new.path);
        END;

        CREATE TRIGGER IF NOT EXISTS packages_ad AFTER DELETE ON packages BEGIN
            INSERT INTO packages_fts(packages_fts, rowid, name, description, path)
            VALUES ('delete', old.rowid, old.name, old.description, old.path);
        END;

        CREATE TRIGGER IF NOT EXISTS packages_au AFTER UPDATE ON packages BEGIN
            INSERT INTO packages_fts(packages_fts, rowid, name, description, path)
            VALUES ('delete', old.rowid, old.name, old.description, old.path);
            INSERT INTO packages_fts(rowid, name, description, path)
            VALUES (new.rowid, new.name, new.description, new.path);
        END;

        CREATE TABLE IF NOT EXISTS shire_meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );
        ",
    )?;
    Ok(())
}

#[cfg(test)]
pub fn create_schema_for_test(conn: &Connection) {
    create_schema(conn).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_schema_creates_tables() {
        let conn = in_memory_db();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(tables.contains(&"packages".to_string()));
        assert!(tables.contains(&"dependencies".to_string()));
        assert!(tables.contains(&"shire_meta".to_string()));
    }

    #[test]
    fn test_insert_and_fts_search() {
        let conn = in_memory_db();
        conn.execute(
            "INSERT INTO packages (name, path, kind, description) VALUES (?1, ?2, ?3, ?4)",
            ("auth-service", "services/auth", "npm", "Authentication and authorization"),
        )
        .unwrap();

        let results: Vec<String> = conn
            .prepare("SELECT name FROM packages_fts WHERE packages_fts MATCH ?1")
            .unwrap()
            .query_map(["auth"], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(results, vec!["auth-service"]);
    }

    #[test]
    fn test_schema_is_idempotent() {
        let conn = in_memory_db();
        create_schema(&conn).unwrap();
    }
}
