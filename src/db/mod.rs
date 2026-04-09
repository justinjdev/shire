pub mod queries;

use anyhow::Result;
use rusqlite::Connection;

pub fn open_or_create(path: &std::path::Path, rag_enabled: bool) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    // Set auto_vacuum before schema creation (must be set on empty DB)
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap_or(0);
    if page_count <= 1 {
        conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")?;
    }
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL;
         PRAGMA cache_size=-64000;
         PRAGMA temp_store=MEMORY;
         PRAGMA mmap_size=268435456;",
    )?;
    create_schema(&conn)?;
    migrate_fts_if_needed(&conn)?;

    #[cfg(feature = "rag")]
    if rag_enabled {
        crate::rag::storage::init_table(&conn)?;
    }

    let _ = rag_enabled;

    Ok(conn)
}

pub fn open_readonly(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA query_only=ON;
         PRAGMA cache_size=-64000;
         PRAGMA temp_store=MEMORY;
         PRAGMA mmap_size=268435456;",
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

        -- Reverse-dep lookup index. The PK leads with `package`, so queries
        -- filtering on `dependency` (package_dependents, reverse_dependency_graph,
        -- change_impact's BFS — the last runs once per BFS node) would full-scan
        -- without this index.
        CREATE INDEX IF NOT EXISTS idx_dependencies_dependency ON dependencies(dependency);

        CREATE VIRTUAL TABLE IF NOT EXISTS packages_fts USING fts5(
            name, description, path,
            content='packages',
            content_rowid='rowid',
            tokenize=\"unicode61 tokenchars '_-'\",
            prefix='2,3'
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

        CREATE TABLE IF NOT EXISTS manifest_hashes (
            path         TEXT PRIMARY KEY,
            content_hash TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS source_hashes (
            package      TEXT PRIMARY KEY,
            content_hash TEXT NOT NULL,
            hashed_at    TEXT
        );

        CREATE TABLE IF NOT EXISTS file_hashes (
            file_path    TEXT NOT NULL,
            package      TEXT NOT NULL REFERENCES packages(name),
            content_hash TEXT NOT NULL,
            hashed_at    TEXT,
            PRIMARY KEY (file_path, package)
        );

        CREATE INDEX IF NOT EXISTS idx_file_hashes_package ON file_hashes(package);

        CREATE TABLE IF NOT EXISTS files (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            path       TEXT NOT NULL UNIQUE,
            package    TEXT REFERENCES packages(name) ON DELETE SET NULL,
            extension  TEXT NOT NULL DEFAULT '',
            size_bytes INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_files_package ON files(package);
        CREATE INDEX IF NOT EXISTS idx_files_extension ON files(extension);

        CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
            path,
            content='files',
            content_rowid='rowid',
            tokenize=\"unicode61 tokenchars '_-'\"
        );

        CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
            INSERT INTO files_fts(rowid, path)
            VALUES (new.rowid, new.path);
        END;

        CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, path)
            VALUES ('delete', old.rowid, old.path);
        END;

        CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, path)
            VALUES ('delete', old.rowid, old.path);
            INSERT INTO files_fts(rowid, path)
            VALUES (new.rowid, new.path);
        END;

        CREATE TABLE IF NOT EXISTS symbols (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            package       TEXT NOT NULL REFERENCES packages(name),
            name          TEXT NOT NULL,
            kind          TEXT NOT NULL,
            signature     TEXT,
            file_path     TEXT NOT NULL,
            line          INTEGER NOT NULL,
            visibility    TEXT NOT NULL DEFAULT 'public',
            parent_symbol TEXT,
            return_type   TEXT,
            parameters    TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_symbols_package ON symbols(package);
        CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
        CREATE INDEX IF NOT EXISTS idx_symbols_file_path ON symbols(file_path);

        CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
            name, kind, signature, file_path,
            content='symbols',
            content_rowid='rowid',
            tokenize=\"unicode61 tokenchars '_'\",
            prefix='2,3'
        );

        CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
            INSERT INTO symbols_fts(rowid, name, kind, signature, file_path)
            VALUES (new.rowid, new.name, new.kind, new.signature, new.file_path);
        END;

        CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
            INSERT INTO symbols_fts(symbols_fts, rowid, name, kind, signature, file_path)
            VALUES ('delete', old.rowid, old.name, old.kind, old.signature, old.file_path);
        END;

        CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
            INSERT INTO symbols_fts(symbols_fts, rowid, name, kind, signature, file_path)
            VALUES ('delete', old.rowid, old.name, old.kind, old.signature, old.file_path);
            INSERT INTO symbols_fts(rowid, name, kind, signature, file_path)
            VALUES (new.rowid, new.name, new.kind, new.signature, new.file_path);
        END;

        CREATE TABLE IF NOT EXISTS docs (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            path         TEXT NOT NULL UNIQUE,
            package      TEXT REFERENCES packages(name) ON DELETE SET NULL,
            title        TEXT,
            body         TEXT NOT NULL,
            size_bytes   INTEGER NOT NULL DEFAULT 0,
            content_hash TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_docs_package ON docs(package);

        CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
            title, body, path,
            content='docs',
            content_rowid='rowid',
            tokenize=\"unicode61 tokenchars '_-'\",
            prefix='2,3'
        );

        CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN
            INSERT INTO docs_fts(rowid, title, body, path)
            VALUES (new.rowid, new.title, new.body, new.path);
        END;

        CREATE TRIGGER IF NOT EXISTS docs_ad AFTER DELETE ON docs BEGIN
            INSERT INTO docs_fts(docs_fts, rowid, title, body, path)
            VALUES ('delete', old.rowid, old.title, old.body, old.path);
        END;

        CREATE TRIGGER IF NOT EXISTS docs_au AFTER UPDATE ON docs BEGIN
            INSERT INTO docs_fts(docs_fts, rowid, title, body, path)
            VALUES ('delete', old.rowid, old.title, old.body, old.path);
            INSERT INTO docs_fts(rowid, title, body, path)
            VALUES (new.rowid, new.title, new.body, new.path);
        END;

        CREATE TABLE IF NOT EXISTS symbol_refs (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            name             TEXT NOT NULL,
            kind             TEXT NOT NULL,
            file_id          INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            line             INTEGER NOT NULL,
            package          TEXT,
            enclosing_symbol TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_refs_name ON symbol_refs(name);
        CREATE INDEX IF NOT EXISTS idx_refs_file_id ON symbol_refs(file_id);
        CREATE INDEX IF NOT EXISTS idx_refs_enclosing ON symbol_refs(enclosing_symbol);
        -- (package, name) composite: supports both per-package deletes
        -- (delete_references_for_package) and name-scoped lookups filtered
        -- by package (query_symbol_references/callers/callees). Without
        -- this, those operations full-scan symbol_refs — at monorepo scale
        -- that is multiple seconds per call.
        CREATE INDEX IF NOT EXISTS idx_refs_package_name ON symbol_refs(package, name);
        -- Covering call-ref indexes used by query_symbol_callers/callees.
        -- Keep them partial (kind='call') so insert/build costs stay bounded.
        CREATE INDEX IF NOT EXISTS idx_refs_callers_cover
            ON symbol_refs(name, enclosing_symbol, package, file_id, line)
            WHERE kind = 'call' AND enclosing_symbol IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_refs_callees_cover
            ON symbol_refs(enclosing_symbol, name, package, file_id, line)
            WHERE kind = 'call';

        CREATE TABLE IF NOT EXISTS boundary_edges (
            source_path       TEXT NOT NULL,
            generated_path    TEXT NOT NULL,
            source_package    TEXT,
            generated_package TEXT,
            kind              TEXT NOT NULL DEFAULT 'proto',
            PRIMARY KEY (source_path, generated_path)
        );

        CREATE INDEX IF NOT EXISTS idx_boundary_source ON boundary_edges(source_path);
        CREATE INDEX IF NOT EXISTS idx_boundary_generated ON boundary_edges(generated_path);
        ",
    )?;
    Ok(())
}

pub fn drop_symbols_fts_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS symbols_ai;
         DROP TRIGGER IF EXISTS symbols_ad;
         DROP TRIGGER IF EXISTS symbols_au;",
    )?;
    Ok(())
}

pub fn recreate_symbols_fts_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
            INSERT INTO symbols_fts(rowid, name, kind, signature, file_path)
            VALUES (new.rowid, new.name, new.kind, new.signature, new.file_path);
        END;
        CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
            INSERT INTO symbols_fts(symbols_fts, rowid, name, kind, signature, file_path)
            VALUES ('delete', old.rowid, old.name, old.kind, old.signature, old.file_path);
        END;
        CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
            INSERT INTO symbols_fts(symbols_fts, rowid, name, kind, signature, file_path)
            VALUES ('delete', old.rowid, old.name, old.kind, old.signature, old.file_path);
            INSERT INTO symbols_fts(rowid, name, kind, signature, file_path)
            VALUES (new.rowid, new.name, new.kind, new.signature, new.file_path);
        END;",
    )?;
    Ok(())
}

pub fn drop_docs_fts_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS docs_ai;
         DROP TRIGGER IF EXISTS docs_ad;
         DROP TRIGGER IF EXISTS docs_au;",
    )?;
    Ok(())
}

pub fn recreate_docs_fts_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN
            INSERT INTO docs_fts(rowid, title, body, path)
            VALUES (new.rowid, new.title, new.body, new.path);
        END;
        CREATE TRIGGER IF NOT EXISTS docs_ad AFTER DELETE ON docs BEGIN
            INSERT INTO docs_fts(docs_fts, rowid, title, body, path)
            VALUES ('delete', old.rowid, old.title, old.body, old.path);
        END;
        CREATE TRIGGER IF NOT EXISTS docs_au AFTER UPDATE ON docs BEGIN
            INSERT INTO docs_fts(docs_fts, rowid, title, body, path)
            VALUES ('delete', old.rowid, old.title, old.body, old.path);
            INSERT INTO docs_fts(rowid, title, body, path)
            VALUES (new.rowid, new.title, new.body, new.path);
        END;",
    )?;
    Ok(())
}

/// Drop the non-FTS indexes on `symbol_refs` before bulk insert.
/// Combined with `recreate_symbol_refs_indexes` after the bulk insert,
/// this moves per-row B-tree updates into one sorted build per index —
/// substantially faster for large batches.
pub fn drop_symbol_refs_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        // idx_refs_file was removed in schema v6: its per-row maintenance
        // cost during INSERT exceeded the savings it gave the per-file
        // DELETE path. Keep the DROP so legacy DBs shed it on next bulk load.
        // In v7 we reintroduce a file index, but on the INTEGER file_id
        // column — its B-tree entries are ~10x smaller than the TEXT-path
        // version, so the INSERT-side cost is proportionally lower.
        "DROP INDEX IF EXISTS idx_refs_name;
         DROP INDEX IF EXISTS idx_refs_file;
         DROP INDEX IF EXISTS idx_refs_file_id;
         DROP INDEX IF EXISTS idx_refs_enclosing;
         DROP INDEX IF EXISTS idx_refs_package_name;
         DROP INDEX IF EXISTS idx_refs_callers_cover;
         DROP INDEX IF EXISTS idx_refs_callees_cover;",
    )?;
    Ok(())
}

pub fn recreate_symbol_refs_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_refs_name ON symbol_refs(name);
         CREATE INDEX IF NOT EXISTS idx_refs_file_id ON symbol_refs(file_id);
         CREATE INDEX IF NOT EXISTS idx_refs_enclosing ON symbol_refs(enclosing_symbol);
         CREATE INDEX IF NOT EXISTS idx_refs_package_name ON symbol_refs(package, name);
         CREATE INDEX IF NOT EXISTS idx_refs_callers_cover
             ON symbol_refs(name, enclosing_symbol, package, file_id, line)
             WHERE kind = 'call' AND enclosing_symbol IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_refs_callees_cover
             ON symbol_refs(enclosing_symbol, name, package, file_id, line)
             WHERE kind = 'call';",
    )?;
    Ok(())
}

/// Read the last-persisted `references_enabled` flag from `shire_meta`.
/// Returns `None` when the key is absent (either a fresh DB or an index
/// built before this key was introduced). Callers treat `None` as "refs
/// are not available" so MCP tools can refuse to serve stale empty
/// `symbol_refs`.
pub fn read_references_enabled(conn: &Connection) -> Option<bool> {
    let v: String = conn
        .query_row(
            "SELECT value FROM shire_meta WHERE key = 'references_enabled'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    match v.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Persist the `references_enabled` flag to `shire_meta`. Called at the
/// end of every build so subsequent opens (including the MCP server) can
/// tell whether `symbol_refs` is populated and in sync with the current
/// source tree.
pub fn write_references_enabled(conn: &Connection, enabled: bool) -> Result<()> {
    let v = if enabled { "true" } else { "false" };
    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('references_enabled', ?1)",
        [v],
    )?;
    Ok(())
}

const FTS_SCHEMA_VERSION: &str = "7";

fn migrate_fts_if_needed(conn: &Connection) -> Result<()> {
    let current: Option<String> = conn
        .query_row(
            "SELECT value FROM shire_meta WHERE key = 'fts_schema_version'",
            [],
            |row| row.get(0),
        )
        .ok();

    if current.as_deref() == Some(FTS_SCHEMA_VERSION) {
        return Ok(());
    }

    tracing::debug!(
        from = ?current.as_deref(),
        to = FTS_SCHEMA_VERSION,
        "migrating FTS schema"
    );

    conn.execute_batch(
        "DROP TRIGGER IF EXISTS packages_ai;
         DROP TRIGGER IF EXISTS packages_ad;
         DROP TRIGGER IF EXISTS packages_au;
         DROP TRIGGER IF EXISTS files_ai;
         DROP TRIGGER IF EXISTS files_ad;
         DROP TRIGGER IF EXISTS files_au;
         DROP TRIGGER IF EXISTS symbols_ai;
         DROP TRIGGER IF EXISTS symbols_ad;
         DROP TRIGGER IF EXISTS symbols_au;
         DROP TRIGGER IF EXISTS docs_ai;
         DROP TRIGGER IF EXISTS docs_ad;
         DROP TRIGGER IF EXISTS docs_au;
         DROP TABLE IF EXISTS packages_fts;
         DROP TABLE IF EXISTS files_fts;
         DROP TABLE IF EXISTS symbols_fts;
         DROP TABLE IF EXISTS docs_fts;
         DROP TRIGGER IF EXISTS symbol_refs_ai;
         DROP TRIGGER IF EXISTS symbol_refs_ad;
         DROP TRIGGER IF EXISTS symbol_refs_au;
         DROP TABLE IF EXISTS symbol_refs_fts;
         DROP INDEX IF EXISTS idx_refs_package;
         DROP INDEX IF EXISTS idx_refs_file;
         -- v7: symbol_refs.file_path TEXT → file_id INTEGER (FK to files(id))
         DROP TABLE IF EXISTS symbol_refs;",
    )?;

    create_schema(conn)?;

    // `symbol_refs` is a regular table with no FTS 'rebuild' hook — it is
    // only populated during symbol extraction. If we just dropped+recreated
    // it (e.g. v7 schema change) but leave `source_hashes`/`file_hashes`
    // intact, the next incremental `shire build` will hash-match every
    // unchanged file, skip extraction, and leave `symbol_refs` empty until
    // the user runs `--force` or edits files. Clear the hash tables so the
    // next build re-extracts.
    conn.execute_batch(
        "DELETE FROM source_hashes;
         DELETE FROM file_hashes;",
    )?;

    conn.execute_batch(
        "INSERT INTO packages_fts(packages_fts) VALUES('rebuild');
         INSERT INTO files_fts(files_fts) VALUES('rebuild');
         INSERT INTO symbols_fts(symbols_fts) VALUES('rebuild');
         INSERT INTO docs_fts(docs_fts) VALUES('rebuild');",
    )?;

    conn.execute(
        "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('fts_schema_version', ?1)",
        [FTS_SCHEMA_VERSION],
    )?;

    Ok(())
}

/// Copy a SQLite database using the backup API.
/// Handles WAL-mode databases correctly. Creates parent directories for `dest`.
/// Uses a temporary file + rename to avoid leaving a partial DB on failure.
pub fn seed_db(source: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    use anyhow::Context;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
    }

    // Write to a temp file in the same directory so rename is atomic.
    let tmp_dest = dest.with_extension("db.seed-tmp");

    let result = (|| -> Result<()> {
        let src_conn = Connection::open_with_flags(
            source,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("Failed to open seed DB '{}'", source.display()))?;
        let mut dst_conn = Connection::open(&tmp_dest)
            .with_context(|| format!("Failed to create DB '{}'", tmp_dest.display()))?;
        let backup =
            rusqlite::backup::Backup::new(&src_conn, &mut dst_conn).with_context(|| {
                format!(
                    "Failed to init backup '{}' -> '{}'",
                    source.display(),
                    dest.display()
                )
            })?;
        backup
            .run_to_completion(100, std::time::Duration::ZERO, None)
            .with_context(|| {
                format!(
                    "Failed to complete backup '{}' -> '{}'",
                    source.display(),
                    dest.display()
                )
            })?;
        Ok(())
    })();

    if let Err(e) = &result {
        let _ = std::fs::remove_file(&tmp_dest);
        return Err(anyhow::anyhow!("{e:#}"));
    }

    std::fs::rename(&tmp_dest, dest).with_context(|| {
        format!(
            "Failed to rename '{}' to '{}'",
            tmp_dest.display(),
            dest.display()
        )
    })?;

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
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(tables.contains(&"packages".to_string()));
        assert!(tables.contains(&"dependencies".to_string()));
        assert!(tables.contains(&"shire_meta".to_string()));
        assert!(tables.contains(&"manifest_hashes".to_string()));
        assert!(tables.contains(&"source_hashes".to_string()));
        assert!(tables.contains(&"files".to_string()));
        assert!(tables.contains(&"symbols".to_string()));
        assert!(tables.contains(&"file_hashes".to_string()));
        assert!(tables.contains(&"docs".to_string()));
    }

    #[test]
    fn test_insert_and_fts_search() {
        let conn = in_memory_db();
        conn.execute(
            "INSERT INTO packages (name, path, kind, description) VALUES (?1, ?2, ?3, ?4)",
            (
                "auth-service",
                "services/auth",
                "npm",
                "Authentication and authorization",
            ),
        )
        .unwrap();

        let results: Vec<String> = conn
            .prepare("SELECT name FROM packages_fts WHERE packages_fts MATCH ?1")
            .unwrap()
            .query_map(["auth"], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(results, vec!["auth-service"]);
    }

    #[test]
    fn test_docs_fts_search() {
        let conn = in_memory_db();
        conn.execute(
            "INSERT INTO docs (path, package, title, body, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "docs/setup.md",
                Option::<String>::None,
                "Getting Started",
                "How to install and configure the service",
                42,
            ),
        )
        .unwrap();

        let results: Vec<String> = conn
            .prepare("SELECT path FROM docs_fts WHERE docs_fts MATCH ?1")
            .unwrap()
            .query_map(["install"], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(results, vec!["docs/setup.md"]);
    }

    #[test]
    fn test_schema_is_idempotent() {
        let conn = in_memory_db();
        create_schema(&conn).unwrap();
    }

    #[test]
    fn test_seed_db_copies_data() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_path = dir.path().join("source.db");
        let src_conn = Connection::open(&src_path).unwrap();
        create_schema(&src_conn).unwrap();
        src_conn
            .execute(
                "INSERT INTO packages (name, path, kind) VALUES (?1, ?2, ?3)",
                ("test-pkg", "packages/test", "npm"),
            )
            .unwrap();
        drop(src_conn);

        let dst_path = dir.path().join("dest.db");
        seed_db(&src_path, &dst_path).unwrap();

        let dst_conn = Connection::open(&dst_path).unwrap();
        let name: String = dst_conn
            .query_row("SELECT name FROM packages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "test-pkg");
    }

    #[test]
    fn test_seed_db_creates_parent_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_path = dir.path().join("source.db");
        let src_conn = Connection::open(&src_path).unwrap();
        create_schema(&src_conn).unwrap();
        drop(src_conn);

        let dst_path = dir.path().join("deep").join("nested").join("dest.db");
        seed_db(&src_path, &dst_path).unwrap();
        assert!(dst_path.exists());
    }

    #[test]
    fn test_seed_db_source_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = seed_db(
            &dir.path().join("nonexistent.db"),
            &dir.path().join("dest.db"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_boundary_edges_table_exists() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO boundary_edges (source_path, generated_path, source_package, generated_package, kind) \
             VALUES ('a.proto', 'a.pb.go', 'pkg', 'pkg', 'proto')",
            [],
        ).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM boundary_edges WHERE source_path = 'a.proto'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_symbol_refs_schema_created() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_or_create(&path, false).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='symbol_refs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "symbol_refs table must exist");
    }

    #[test]
    fn test_symbol_refs_insert_and_query() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_or_create(&path, false).unwrap();

        conn.execute(
            "INSERT INTO files (path, package, extension, size_bytes) VALUES ('src/main.rs', NULL, 'rs', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn
            .query_row("SELECT id FROM files WHERE path = 'src/main.rs'", [], |r| {
                r.get(0)
            })
            .unwrap();
        conn.execute(
            "INSERT INTO symbol_refs (name, kind, file_id, line, package, enclosing_symbol) \
             VALUES ('parseConfig', 'call', ?1, 42, NULL, 'handle_request')",
            [file_id],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // idx_refs_name should support exact-name lookups used by MCP tools
        let hit: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbol_refs WHERE name = 'parseConfig'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hit, 1);
    }

    #[test]
    fn test_idx_refs_package_name_exists() {
        // Package-scoped ref queries (query_symbol_references/callers/callees
        // with a package filter, delete_references_for_package) must not
        // full-scan symbol_refs. The (package, name) composite index was
        // missing for a stretch — verify it's created for fresh DBs.
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_or_create(&path, false).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_refs_package_name'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "idx_refs_package_name must exist");
    }

    #[test]
    fn test_call_ref_covering_indexes_exist() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_or_create(&path, false).unwrap();

        let callers_idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_refs_callers_cover'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let callees_idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_refs_callees_cover'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        assert_eq!(callers_idx, 1, "idx_refs_callers_cover must exist");
        assert_eq!(callees_idx, 1, "idx_refs_callees_cover must exist");
    }

    #[test]
    fn test_references_enabled_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_or_create(&path, false).unwrap();

        assert_eq!(
            read_references_enabled(&conn),
            None,
            "fresh DB has no persisted flag"
        );

        write_references_enabled(&conn, true).unwrap();
        assert_eq!(read_references_enabled(&conn), Some(true));

        write_references_enabled(&conn, false).unwrap();
        assert_eq!(read_references_enabled(&conn), Some(false));
    }
}
