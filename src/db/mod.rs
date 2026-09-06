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
    apply_schema(&conn)?;

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

/// The one and only definition of `symbols_fts`.
///
/// `create_schema` and the phase-7 bulk rebuild in `index::phase_extract_symbols`
/// both create this table; before this const existed the two sites disagreed on
/// the tokenizer and on `prefix=`, so the tokenization of an index depended on
/// which code path last created the table.
///
/// `name_tokens` holds the sub-tokens of `name` (see [`split_identifier`]) so
/// that a query for one part of a camelCase/snake_case identifier matches the
/// whole identifier.
pub const SYMBOLS_FTS_DDL: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name, kind, signature, file_path, name_tokens,
    content='symbols',
    content_rowid='rowid',
    tokenize=\"unicode61 tokenchars '_-'\",
    prefix='2,3'
);";

/// Triggers keeping `symbols_fts` in sync with `symbols`. Shared by
/// `create_schema` and [`recreate_symbols_fts_triggers`] so the column list
/// only has to be right once.
pub const SYMBOLS_FTS_TRIGGERS: &str =
    "CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
        INSERT INTO symbols_fts(rowid, name, kind, signature, file_path, name_tokens)
        VALUES (new.rowid, new.name, new.kind, new.signature, new.file_path, new.name_tokens);
    END;
    CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
        INSERT INTO symbols_fts(symbols_fts, rowid, name, kind, signature, file_path, name_tokens)
        VALUES ('delete', old.rowid, old.name, old.kind, old.signature, old.file_path, old.name_tokens);
    END;
    CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
        INSERT INTO symbols_fts(symbols_fts, rowid, name, kind, signature, file_path, name_tokens)
        VALUES ('delete', old.rowid, old.name, old.kind, old.signature, old.file_path, old.name_tokens);
        INSERT INTO symbols_fts(rowid, name, kind, signature, file_path, name_tokens)
        VALUES (new.rowid, new.name, new.kind, new.signature, new.file_path, new.name_tokens);
    END;";

/// Split an identifier into its lowercase sub-tokens, space separated:
/// `verifyJwtToken` -> `"verify jwt token"`, `handle_rate_limit` ->
/// `"handle rate limit"`, `HTTPServer` -> `"http server"`.
///
/// Returns an empty string only when the split adds nothing the `name`
/// column already indexes — i.e. when it yields exactly the lowercased name.
/// A leading or trailing separator does change the term (`_helper` is one
/// `name` term because `_` is a tokenchar, and `"helper"*` does not match
/// it), so those still get a `name_tokens` entry.
///
/// Called once per symbol from the batched INSERT in `index`. It allocates a
/// few small buffers per call (no regex, no shared state); measured at ~2% of
/// full-build time on a real repository.
pub fn split_identifier(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            // `_`, `-`, `.`, `:` … all act as separators.
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        // camelCase boundary: `parseJwt` -> parse|Jwt, and the tail of an
        // acronym run: `HTTPServer` -> HTTP|Server. Digits stay attached to
        // the word before them (`sha256` stays one token).
        let boundary = match prev {
            Some(p) if c.is_uppercase() => {
                p.is_lowercase()
                    || p.is_numeric()
                    || (p.is_uppercase() && next.is_some_and(|n| n.is_lowercase()))
            }
            _ => false,
        };
        if boundary && !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    let joined = tokens.join(" ");
    // Nothing gained when the split is the name itself: `handle` is already
    // one `name` term. `_helper` is not — its `name` term keeps the
    // underscore — so it does get an entry.
    if joined == name.to_lowercase() {
        return String::new();
    }
    joined
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

        -- `name_tokens` holds the sub-tokens of `name`, space separated and
        -- lowercased (`verifyJwtToken` -> `verify jwt token`); symbols_fts
        -- indexes it so a query for one part of an identifier matches the
        -- whole identifier. NULL/empty when the name has no sub-tokens — the
        -- `name` column already indexes that term.
        -- (Comments stay outside the CREATE TABLE body: SQLite re-parses the
        -- stored SQL during ALTER TABLE and chokes on comments inside it.)
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
            parameters    TEXT,
            name_tokens   TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_symbols_package ON symbols(package);
        CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
        CREATE INDEX IF NOT EXISTS idx_symbols_file_path ON symbols(file_path);

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
    // symbols_fts and its triggers live in shared consts because the bulk
    // rebuild in `index::phase_extract_symbols` recreates the same table.
    conn.execute_batch(SYMBOLS_FTS_DDL)?;
    conn.execute_batch(SYMBOLS_FTS_TRIGGERS)?;
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
    conn.execute_batch(SYMBOLS_FTS_TRIGGERS)?;
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

/// Version of the *derived* schema: the FTS tables plus any table whose
/// column layout changes between releases. Bumping it makes the next open of
/// an existing index run [`migrate_schema`], which drops and rebuilds those
/// tables from the content tables.
///
/// v8: `symbols.name_tokens` + `symbols_fts.name_tokens` (identifier
/// sub-token index), and a single `symbols_fts` definition shared with the
/// build's bulk rebuild (tokenchars `'_-'` + `prefix='2,3'`).
const FTS_SCHEMA_VERSION: &str = "8";

fn read_fts_schema_version(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT value FROM shire_meta WHERE key = 'fts_schema_version'",
        [],
        |row| row.get(0),
    )
    .ok()
}

/// Create the schema, migrating an existing database first when its derived
/// schema version is stale.
///
/// Ordering matters: the destructive half of a migration runs *before*
/// `create_schema`. `CREATE TABLE IF NOT EXISTS` is a no-op against a table
/// whose columns changed, so any `CREATE INDEX`/`CREATE TRIGGER` naming a new
/// column would fail against the old layout (this is exactly how the v7
/// `symbol_refs.file_path` -> `file_id` change failed). Dropping and altering
/// up front keeps column-changing migrations possible.
fn apply_schema(conn: &Connection) -> Result<()> {
    let current = read_fts_schema_version(conn);
    let migrating = current.as_deref() != Some(FTS_SCHEMA_VERSION);

    if migrating {
        migrate_pre_create(conn)?;
    }

    create_schema(conn)?;

    if migrating {
        tracing::debug!(
            from = ?current.as_deref(),
            to = FTS_SCHEMA_VERSION,
            "migrating derived schema"
        );
        migrate_post_create(conn)?;
    }

    Ok(())
}

/// Destructive half of a migration: everything that must happen before
/// `create_schema` runs. Safe on a fresh database (every statement is
/// `IF EXISTS`).
fn migrate_pre_create(conn: &Connection) -> Result<()> {
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
         -- v7: symbol_refs.file_path TEXT → file_id INTEGER (FK to files(id)).
         -- symbol_refs carries no data that cannot be re-extracted, so the
         -- table is dropped rather than rewritten.
         DROP TABLE IF EXISTS symbol_refs;",
    )?;

    // v8: `symbols` gained `name_tokens`. `symbols` *does* carry data we do
    // not want to throw away (an index that is not rebuilt yet still has to
    // answer queries), so add the column in place and backfill below.
    add_column_if_missing(conn, "symbols", "name_tokens", "TEXT")?;

    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` when `table` exists and lacks `column`.
/// A no-op on a fresh database, where `create_schema` creates the column.
fn add_column_if_missing(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<bool> {
    let table_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get::<_, i64>(0),
    )? > 0;
    if !table_exists {
        return Ok(false);
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<_, _>>()?;
    if existing.iter().any(|c| c == column) {
        return Ok(false);
    }
    conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))?;
    Ok(true)
}

/// Fill in `symbols.name_tokens` for rows that predate the column. Runs once,
/// right after the migration adds it; normal builds populate the column in the
/// batched INSERT instead.
fn backfill_name_tokens(conn: &Connection) -> Result<usize> {
    let pending: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT rowid, name FROM symbols WHERE name_tokens IS NULL")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    if pending.is_empty() {
        return Ok(0);
    }
    // One transaction for the whole backfill: per-row autocommit would mean
    // one WAL commit per symbol, which is minutes on a large index.
    let tx = conn.unchecked_transaction()?;
    {
        let mut upd = tx.prepare("UPDATE symbols SET name_tokens = ?2 WHERE rowid = ?1")?;
        for (rowid, name) in &pending {
            upd.execute(rusqlite::params![rowid, split_identifier(name)])?;
        }
    }
    tx.commit()?;
    Ok(pending.len())
}

/// Non-destructive half of a migration: runs after `create_schema` has
/// recreated the dropped tables.
fn migrate_post_create(conn: &Connection) -> Result<()> {
    // Backfill with the FTS triggers off: symbols_fts has just been created
    // and is still empty, so the UPDATE's `symbols_au` trigger would issue a
    // 'delete' for rows the index has never seen, which corrupts an
    // external-content FTS table. The bulk 'rebuild' below fills it instead.
    drop_symbols_fts_triggers(conn)?;
    let backfilled = backfill_name_tokens(conn)?;
    recreate_symbols_fts_triggers(conn)?;
    if backfilled > 0 {
        tracing::debug!(rows = backfilled, "backfilled symbols.name_tokens");
    }

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

    // ── Identifier sub-tokens (search for part of a camelCase name) ──────

    #[test]
    fn test_split_identifier_camel_case() {
        assert_eq!(split_identifier("verifyJwtToken"), "verify jwt token");
        assert_eq!(split_identifier("parseConfig"), "parse config");
        assert_eq!(split_identifier("AuthMiddleware"), "auth middleware");
    }

    #[test]
    fn test_split_identifier_snake_and_kebab() {
        assert_eq!(split_identifier("check_rate_limit"), "check rate limit");
        assert_eq!(split_identifier("token-store"), "token store");
        assert_eq!(split_identifier("handle_a_0_1"), "handle a 0 1");
    }

    #[test]
    fn test_split_identifier_acronyms_and_digits() {
        // Trailing letter of an acronym run starts the next word.
        assert_eq!(split_identifier("HTTPServer"), "http server");
        assert_eq!(split_identifier("parseHTTPHeader"), "parse http header");
        // Digits stay attached to the word they follow.
        assert_eq!(split_identifier("sha256Hash"), "sha256 hash");
    }

    /// A leading or trailing separator stays part of the `name` term
    /// (`_` and `-` are tokenchars), so `"helper"*` cannot reach `_helper` —
    /// these names need a sub-token entry even though they are one word.
    #[test]
    fn test_split_identifier_keeps_names_with_leading_separators() {
        assert_eq!(split_identifier("_helper"), "helper");
        assert_eq!(split_identifier("__init__"), "init");
        assert_eq!(split_identifier("-flag"), "flag");
    }

    #[test]
    fn test_split_identifier_single_word_is_empty() {
        // Nothing to add: the FTS `name` column already indexes this term,
        // so storing a copy would only cost space.
        assert_eq!(split_identifier("handle"), "");
        assert_eq!(split_identifier("Config"), "");
        assert_eq!(split_identifier(""), "");
        assert_eq!(split_identifier("__"), "");
    }

    // ── DB-4: one definition of symbols_fts, used by both creators ───────

    #[test]
    fn test_symbols_fts_ddl_is_the_only_definition() {
        // The build's phase-7 bulk rebuild drops and recreates symbols_fts.
        // It must use the same const, or the tokenization of an index would
        // depend on which code path last created the table.
        let index_src = include_str!("../index/mod.rs");
        assert!(
            index_src.contains("db::SYMBOLS_FTS_DDL"),
            "index/mod.rs must recreate symbols_fts from db::SYMBOLS_FTS_DDL"
        );
        assert!(
            !index_src.contains("CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts"),
            "index/mod.rs must not carry its own symbols_fts DDL"
        );
    }

    #[test]
    fn test_symbols_fts_rebuild_matches_created_schema() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        let fts_sql = |c: &Connection| -> String {
            c.query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'symbols_fts'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };
        let created = fts_sql(&conn);
        // Simulate the phase-7 rebuild.
        conn.execute_batch("DROP TABLE symbols_fts;").unwrap();
        conn.execute_batch(SYMBOLS_FTS_DDL).unwrap();
        let rebuilt = fts_sql(&conn);
        assert_eq!(created, rebuilt);
        assert!(
            rebuilt.contains("prefix='2,3'"),
            "prefix index kept: {rebuilt}"
        );
        assert!(
            rebuilt.contains("name_tokens"),
            "sub-token column: {rebuilt}"
        );
    }

    // ── Migration ────────────────────────────────────────────────────────

    /// Write a database with the shape schema v7 had: no
    /// `symbols.name_tokens`, a four-column `symbols_fts` tokenized with
    /// `'_'` only. `create_schema` fills in everything else on open.
    fn write_v7_db(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE shire_meta (key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO shire_meta (key, value) VALUES ('fts_schema_version', '7');
             CREATE TABLE packages (
                 name TEXT PRIMARY KEY, path TEXT NOT NULL UNIQUE, kind TEXT NOT NULL,
                 version TEXT, description TEXT, metadata TEXT);
             INSERT INTO packages (name, path, kind) VALUES ('p', 'p', 'npm');
             CREATE TABLE symbols (
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
                 parameters    TEXT);
             INSERT INTO symbols (package, name, kind, file_path, line)
                 VALUES ('p', 'verifyJwtToken', 'function', 'p/a.ts', 1);
             CREATE VIRTUAL TABLE symbols_fts USING fts5(
                 name, kind, signature, file_path,
                 content='symbols',
                 content_rowid='rowid',
                 tokenize=\"unicode61 tokenchars '_'\"
             );
             INSERT INTO symbols_fts(symbols_fts) VALUES('rebuild');",
        )
        .unwrap();
    }

    #[test]
    fn test_migration_v7_adds_and_backfills_name_tokens() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("v7.db");
        write_v7_db(&path);

        // Reopening runs the migration.
        let conn = open_or_create(&path, false).unwrap();
        assert_eq!(
            read_fts_schema_version(&conn).as_deref(),
            Some(FTS_SCHEMA_VERSION)
        );
        let tokens: String = conn
            .query_row("SELECT name_tokens FROM symbols", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tokens, "verify jwt token", "backfilled from the name");
        // The rebuilt FTS index carries the sub-tokens, so a query for one
        // part of the identifier now matches.
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbols_fts WHERE symbols_fts MATCH '\"jwt\"*'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "sub-token search hits after migration");
        // The rebuilt table is the shared definition, prefix index included.
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'symbols_fts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sql.contains("prefix='2,3'"), "got {sql}");
    }

    /// DB-V1: a column-changing migration used to fail because
    /// `create_schema` ran first — `CREATE TABLE IF NOT EXISTS` left the old
    /// `symbol_refs` in place and the following `CREATE INDEX ... (file_id)`
    /// errored with "no such column: file_id" before the drop could run.
    #[test]
    fn test_migration_from_pre_v7_symbol_refs_layout() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("v6.db");
        {
            let conn = open_or_create(&path, false).unwrap();
            conn.execute_batch(
                "DROP TABLE IF EXISTS symbol_refs;
                 CREATE TABLE symbol_refs (
                     id               INTEGER PRIMARY KEY,
                     name             TEXT NOT NULL,
                     kind             TEXT NOT NULL,
                     package          TEXT NOT NULL,
                     file_path        TEXT NOT NULL,
                     line             INTEGER NOT NULL,
                     enclosing_symbol TEXT
                 );",
            )
            .unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('fts_schema_version', '6')",
                [],
            )
            .unwrap();
        }

        let conn = open_or_create(&path, false).expect("v6 layout must migrate, not error");
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(symbol_refs)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(cols.contains(&"file_id".to_string()), "got {cols:?}");
        assert!(!cols.contains(&"file_path".to_string()), "got {cols:?}");
        assert_eq!(
            read_fts_schema_version(&conn).as_deref(),
            Some(FTS_SCHEMA_VERSION)
        );
    }

    #[test]
    fn test_migration_is_skipped_when_version_matches() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("current.db");
        {
            let conn = open_or_create(&path, false).unwrap();
            conn.execute(
                "INSERT INTO source_hashes (package, content_hash) VALUES ('p', 'h')",
                [],
            )
            .unwrap();
        }
        // Reopening at the current version must not clear the hash tables
        // (that would force a full re-extraction on every open).
        let conn = open_or_create(&path, false).unwrap();
        let hashes: i64 = conn
            .query_row("SELECT COUNT(*) FROM source_hashes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hashes, 1);
    }
}
