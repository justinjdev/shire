# RAG Search Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add optional vector similarity search to `search_symbols` using local embeddings, so natural language queries return semantically relevant results even without keyword overlap.

**Architecture:** `fastembed` generates 384-dim embeddings for each symbol at index time. Vectors are stored in the existing SQLite DB via `sqlite-vec`. At query time, `search_symbols` runs both FTS5 and vector search, merging results with Reciprocal Rank Fusion (RRF). All RAG code is behind a `rag` Cargo feature flag and `[rag]` config section.

**Tech Stack:** Rust, fastembed (ONNX Runtime), sqlite-vec, SQLite

**Design doc:** `openspec/changes/rag-search/design.md`

---

### Task 1: Add Cargo feature flag and dependencies

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add optional dependencies and feature flag**

Add to `Cargo.toml`:

```toml
[features]
default = []
rag = ["dep:fastembed", "dep:sqlite-vec"]

[dependencies]
fastembed = { version = "4", optional = true }
sqlite-vec = { version = "0.1", optional = true }
```

Note: Check crates.io for the actual latest stable versions of both crates at implementation time. `fastembed` may be at v4 or v5, and `sqlite-vec` may have a different version string. Use whatever is current.

**Step 2: Verify compilation without feature**

Run: `cargo check`
Expected: Compiles successfully, no changes to default behavior.

**Step 3: Verify compilation with feature**

Run: `cargo check --features rag`
Expected: Compiles successfully (no code uses the deps yet, but they should resolve).

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(rag): add fastembed and sqlite-vec optional dependencies"
```

---

### Task 2: Add `[rag]` config parsing

**Files:**
- Modify: `src/config.rs`

**Step 1: Write failing test**

Add to the `#[cfg(test)] mod tests` block in `src/config.rs`:

```rust
#[test]
fn test_parse_rag_config_enabled() {
    let toml_str = r#"
[rag]
enabled = true
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert!(config.rag.enabled);
    assert!(config.rag.model.is_none());
    assert!(config.rag.cache_dir.is_none());
}

#[test]
fn test_parse_rag_config_with_options() {
    let toml_str = r#"
[rag]
enabled = true
model = "BAAI/bge-small-en-v1.5"
cache_dir = "/tmp/shire-models"
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert!(config.rag.enabled);
    assert_eq!(config.rag.model.as_deref(), Some("BAAI/bge-small-en-v1.5"));
    assert_eq!(config.rag.cache_dir.as_deref(), Some("/tmp/shire-models"));
}

#[test]
fn test_parse_rag_config_default() {
    let config = Config::default();
    assert!(!config.rag.enabled);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test config::tests::test_parse_rag_config -- --no-capture`
Expected: FAIL — `Config` has no `rag` field.

**Step 3: Add RagConfig struct and wire into Config**

Add above the `Config` struct in `src/config.rs`:

```rust
#[derive(Debug, Deserialize, Default, Clone)]
pub struct RagConfig {
    #[serde(default)]
    pub enabled: bool,
    pub model: Option<String>,
    pub cache_dir: Option<String>,
}
```

Add to the `Config` struct:

```rust
#[serde(default)]
pub rag: RagConfig,
```

**Step 4: Run tests to verify they pass**

Run: `cargo test config::tests`
Expected: All tests pass, including the 3 new ones.

**Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(rag): add [rag] config section parsing"
```

---

### Task 3: Create `src/rag/` module with embedder

**Files:**
- Create: `src/rag/mod.rs`
- Create: `src/rag/embedder.rs`
- Modify: `src/main.rs` (add `mod rag;`)

**Step 1: Write failing test for text representation**

Create `src/rag/embedder.rs`:

```rust
use crate::db::queries::SymbolRow;

/// Build the text representation of a symbol for embedding.
pub fn symbol_to_text(sym: &SymbolRow) -> String {
    // Will implement after test
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_symbol(name: &str, kind: &str, sig: Option<&str>, pkg: &str, path: &str) -> SymbolRow {
        SymbolRow {
            name: name.to_string(),
            kind: kind.to_string(),
            signature: sig.map(|s| s.to_string()),
            package: pkg.to_string(),
            file_path: path.to_string(),
            line: 1,
            visibility: "public".to_string(),
            parent_symbol: None,
            return_type: None,
            parameters: None,
        }
    }

    #[test]
    fn test_symbol_to_text_with_signature() {
        let sym = make_symbol(
            "authenticate",
            "function",
            Some("fn authenticate(req: Request, key: ApiKey) -> Result<Token>"),
            "auth-service",
            "src/auth/middleware.rs",
        );
        let text = symbol_to_text(&sym);
        assert_eq!(
            text,
            "function authenticate in auth-service — fn authenticate(req: Request, key: ApiKey) -> Result<Token> @ src/auth/middleware.rs"
        );
    }

    #[test]
    fn test_symbol_to_text_without_signature() {
        let sym = make_symbol(
            "UserConfig",
            "struct",
            None,
            "shared-types",
            "src/types.ts",
        );
        let text = symbol_to_text(&sym);
        assert_eq!(
            text,
            "struct UserConfig in shared-types @ src/types.ts"
        );
    }
}
```

Create `src/rag/mod.rs`:

```rust
#[cfg(feature = "rag")]
pub mod embedder;
```

Add to `src/main.rs` (with the other `mod` declarations):

```rust
mod rag;
```

**Step 2: Run test to verify it fails**

Run: `cargo test --features rag rag::embedder::tests`
Expected: FAIL — `todo!()` panics.

**Step 3: Implement symbol_to_text**

Replace the `todo!()` in `symbol_to_text`:

```rust
pub fn symbol_to_text(sym: &SymbolRow) -> String {
    match &sym.signature {
        Some(sig) => format!(
            "{} {} in {} — {} @ {}",
            sym.kind, sym.name, sym.package, sig, sym.file_path
        ),
        None => format!(
            "{} {} in {} @ {}",
            sym.kind, sym.name, sym.package, sym.file_path
        ),
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --features rag rag::embedder::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/rag/ src/main.rs
git commit -m "feat(rag): add rag module with symbol text representation"
```

---

### Task 4: Add fastembed embedding wrapper

**Files:**
- Modify: `src/rag/embedder.rs`

This task adds the actual embedding functionality using fastembed. Since fastembed downloads a model on first use (~33MB), tests that call `embed_batch` are integration-level and should be behind a `#[ignore]` attribute (run manually, not in CI without the model).

**Step 1: Write the embedding wrapper**

Add to `src/rag/embedder.rs`:

```rust
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use anyhow::Result;

pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    /// Initialize the embedding model. Downloads on first use.
    pub fn new(model_name: Option<&str>, cache_dir: Option<&str>) -> Result<Self> {
        let mut options = InitOptions::new(EmbeddingModel::BGESmallENV15);

        if let Some(dir) = cache_dir {
            options = options.with_cache_dir(dir.into());
        }

        // If a custom model name is provided, this is where we'd handle it.
        // For now we only support the default BGE model.
        if let Some(name) = model_name {
            eprintln!("[rag] Custom model '{}' requested — using default BGE-small for now", name);
        }

        eprintln!("[rag] Initializing embedding model...");
        let model = TextEmbedding::try_new(options)?;
        Ok(Self { model })
    }

    /// Embed a batch of text strings. Returns one vector per input.
    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let embeddings = self.model.embed(texts.to_vec(), None)?;
        Ok(embeddings)
    }

    /// Embed a single query string.
    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.model.embed(vec![text.to_string()], None)?;
        results.into_iter().next().ok_or_else(|| anyhow::anyhow!("empty embedding result"))
    }
}
```

Note: The fastembed API may differ slightly from what's shown. At implementation time, check the actual fastembed crate docs for the correct constructor, options, and method signatures. The `EmbeddingModel` enum variant name, `InitOptions` builder pattern, and `embed()` signature may vary between versions.

**Step 2: Write an ignored integration test**

Add to the tests module in `embedder.rs`:

```rust
#[test]
#[ignore] // Requires model download (~33MB)
fn test_embed_batch_produces_vectors() {
    let embedder = Embedder::new(None, None).expect("model init failed");
    let texts = vec![
        "function authenticate in auth-service".to_string(),
        "struct UserConfig in shared-types".to_string(),
    ];
    let vectors = embedder.embed_batch(&texts).unwrap();
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].len(), 384); // BGE-small dimension
    assert_eq!(vectors[1].len(), 384);
}

#[test]
#[ignore]
fn test_embed_query() {
    let embedder = Embedder::new(None, None).expect("model init failed");
    let vec = embedder.embed_query("find authentication handler").unwrap();
    assert_eq!(vec.len(), 384);
}
```

**Step 3: Verify it compiles**

Run: `cargo check --features rag`
Expected: Compiles. (Don't run ignored tests yet unless you want to download the model.)

**Step 4: Commit**

```bash
git add src/rag/embedder.rs
git commit -m "feat(rag): add fastembed embedding wrapper"
```

---

### Task 5: Create vector storage layer

**Files:**
- Create: `src/rag/storage.rs`
- Modify: `src/rag/mod.rs`

**Step 1: Write failing test for vector storage**

Create `src/rag/storage.rs`:

```rust
use anyhow::Result;
use rusqlite::Connection;

/// Load the sqlite-vec extension into a connection.
pub fn load_vec_extension(conn: &Connection) -> Result<()> {
    todo!()
}

/// Create the symbol_embeddings virtual table.
pub fn create_embeddings_table(conn: &Connection) -> Result<()> {
    todo!()
}

/// Insert a batch of embeddings. Each entry is (symbol_id, vector).
pub fn insert_embeddings(conn: &Connection, embeddings: &[(i64, Vec<f32>)]) -> Result<()> {
    todo!()
}

/// Delete embeddings for the given symbol IDs.
pub fn delete_embeddings(conn: &Connection, symbol_ids: &[i64]) -> Result<()> {
    todo!()
}

/// Query the top-K most similar vectors. Returns (symbol_id, distance) pairs.
pub fn search_similar(conn: &Connection, query_vec: &[f32], limit: usize) -> Result<Vec<(i64, f64)>> {
    todo!()
}

/// Check if the embeddings table exists and has data.
pub fn has_embeddings(conn: &Connection) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        crate::db::create_schema_for_test(&conn);
        load_vec_extension(&conn).unwrap();
        create_embeddings_table(&conn).unwrap();
        conn
    }

    #[test]
    fn test_insert_and_search() {
        let conn = setup_db();

        // Insert a symbol so we have a valid ID
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('pkg', 'pkg/', 'npm')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO symbols (package, name, kind, file_path, line) VALUES ('pkg', 'foo', 'function', 'foo.ts', 1)",
            [],
        ).unwrap();
        let sym_id: i64 = conn.query_row(
            "SELECT id FROM symbols WHERE name = 'foo'", [], |r| r.get(0)
        ).unwrap();

        // Create a simple 384-dim vector
        let vec384: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
        insert_embeddings(&conn, &[(sym_id, vec384.clone())]).unwrap();

        assert!(has_embeddings(&conn));

        // Search with the same vector should return the symbol
        let results = search_similar(&conn, &vec384, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, sym_id);
    }

    #[test]
    fn test_delete_embeddings() {
        let conn = setup_db();

        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('pkg', 'pkg/', 'npm')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO symbols (package, name, kind, file_path, line) VALUES ('pkg', 'bar', 'function', 'bar.ts', 1)",
            [],
        ).unwrap();
        let sym_id: i64 = conn.query_row(
            "SELECT id FROM symbols WHERE name = 'bar'", [], |r| r.get(0)
        ).unwrap();

        let vec384: Vec<f32> = vec![0.5; 384];
        insert_embeddings(&conn, &[(sym_id, vec384.clone())]).unwrap();
        assert!(has_embeddings(&conn));

        delete_embeddings(&conn, &[sym_id]).unwrap();
        let results = search_similar(&conn, &vec384, 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_has_embeddings_empty() {
        let conn = setup_db();
        assert!(!has_embeddings(&conn));
    }
}
```

Update `src/rag/mod.rs`:

```rust
#[cfg(feature = "rag")]
pub mod embedder;
#[cfg(feature = "rag")]
pub mod storage;
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --features rag rag::storage::tests`
Expected: FAIL — all functions are `todo!()`.

**Step 3: Implement the storage functions**

Replace the `todo!()` implementations in `src/rag/storage.rs`. The actual sqlite-vec API needs to be verified at implementation time — check the `sqlite-vec` crate docs for:
- How to load the extension (`sqlite_vec::sqlite3_vec_init` or similar)
- The `vec0` virtual table syntax
- How to insert and query vectors (may use JSON serialization or binary format)
- The distance function name (`vec_distance_cosine` or similar)

The general shape:

```rust
pub fn load_vec_extension(conn: &Connection) -> Result<()> {
    // Use sqlite_vec's provided init function
    // e.g., sqlite_vec::load(conn)?;
    todo!("Check sqlite-vec crate docs for exact loading API")
}

pub fn create_embeddings_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS symbol_embeddings USING vec0(
            symbol_id INTEGER PRIMARY KEY,
            embedding FLOAT[384]
        );"
    )?;
    Ok(())
}

pub fn insert_embeddings(conn: &Connection, embeddings: &[(i64, Vec<f32>)]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO symbol_embeddings (symbol_id, embedding) VALUES (?1, ?2)"
    )?;
    for (id, vec) in embeddings {
        // sqlite-vec may need the vector as JSON array or binary blob
        // Check docs for the correct serialization format
        stmt.execute(rusqlite::params![id, serde_json::to_string(vec)?])?;
    }
    Ok(())
}

pub fn delete_embeddings(conn: &Connection, symbol_ids: &[i64]) -> Result<()> {
    for id in symbol_ids {
        conn.execute("DELETE FROM symbol_embeddings WHERE symbol_id = ?1", [id])?;
    }
    Ok(())
}

pub fn search_similar(conn: &Connection, query_vec: &[f32], limit: usize) -> Result<Vec<(i64, f64)>> {
    // sqlite-vec query syntax — verify against docs
    let query_json = serde_json::to_string(query_vec)?;
    let mut stmt = conn.prepare(
        "SELECT symbol_id, distance
         FROM symbol_embeddings
         WHERE embedding MATCH ?1
         ORDER BY distance
         LIMIT ?2"
    )?;
    let rows = stmt.query_map(rusqlite::params![query_json, limit as i64], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
}

pub fn has_embeddings(conn: &Connection) -> bool {
    conn.query_row("SELECT COUNT(*) FROM symbol_embeddings", [], |r| r.get::<_, i64>(0))
        .map(|c| c > 0)
        .unwrap_or(false)
}
```

**Important:** The sqlite-vec API is pre-v1 and may differ from what's shown. Read the crate docs and examples before implementing. The vector serialization format (JSON vs binary), distance function, and query syntax are the key things to verify.

**Step 4: Run tests to verify they pass**

Run: `cargo test --features rag rag::storage::tests`
Expected: All 3 tests pass.

**Step 5: Commit**

```bash
git add src/rag/storage.rs src/rag/mod.rs
git commit -m "feat(rag): add sqlite-vec vector storage layer"
```

---

### Task 6: Create public RAG API

**Files:**
- Modify: `src/rag/mod.rs`

**Step 1: Write the public API**

Replace `src/rag/mod.rs` with:

```rust
#[cfg(feature = "rag")]
pub mod embedder;
#[cfg(feature = "rag")]
pub mod storage;

#[cfg(feature = "rag")]
use crate::config::RagConfig;
#[cfg(feature = "rag")]
use crate::db::queries::SymbolRow;
#[cfg(feature = "rag")]
use anyhow::Result;
#[cfg(feature = "rag")]
use rusqlite::Connection;

/// Initialize RAG storage (create tables, load extension).
/// Call this during DB setup when RAG is enabled.
#[cfg(feature = "rag")]
pub fn init_storage(conn: &Connection) -> Result<()> {
    storage::load_vec_extension(conn)?;
    storage::create_embeddings_table(conn)?;
    Ok(())
}

/// Generate and store embeddings for a list of symbols.
/// Skips individual symbols that fail to embed (logs warning).
#[cfg(feature = "rag")]
pub fn embed_symbols(
    conn: &Connection,
    symbols: &[SymbolRow],
    symbol_ids: &[i64],
    config: &RagConfig,
) -> Result<()> {
    if symbols.is_empty() {
        return Ok(());
    }

    let emb = embedder::Embedder::new(
        config.model.as_deref(),
        config.cache_dir.as_deref(),
    )?;

    let texts: Vec<String> = symbols.iter().map(|s| embedder::symbol_to_text(s)).collect();

    eprintln!("[rag] Generating embeddings for {} symbols...", texts.len());
    let vectors = emb.embed_batch(&texts)?;

    let pairs: Vec<(i64, Vec<f32>)> = symbol_ids.iter()
        .copied()
        .zip(vectors.into_iter())
        .collect();

    storage::insert_embeddings(conn, &pairs)?;
    eprintln!("[rag] Stored {} embeddings", pairs.len());

    Ok(())
}

/// Delete embeddings for symbols belonging to a package.
#[cfg(feature = "rag")]
pub fn delete_package_embeddings(conn: &Connection, package: &str) -> Result<()> {
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM symbols WHERE package = ?1")?
        .query_map([package], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    if !ids.is_empty() {
        storage::delete_embeddings(conn, &ids)?;
    }
    Ok(())
}

/// Search for similar symbols by embedding the query text.
/// Returns symbol IDs ranked by similarity.
#[cfg(feature = "rag")]
pub fn search_similar(
    conn: &Connection,
    query: &str,
    limit: usize,
    config: &RagConfig,
) -> Result<Vec<(i64, f64)>> {
    if !storage::has_embeddings(conn) {
        return Ok(Vec::new());
    }

    let emb = embedder::Embedder::new(
        config.model.as_deref(),
        config.cache_dir.as_deref(),
    )?;
    let query_vec = emb.embed_query(query)?;
    storage::search_similar(conn, &query_vec, limit)
}

/// Check if RAG embeddings are available.
#[cfg(feature = "rag")]
pub fn is_available(conn: &Connection) -> bool {
    storage::has_embeddings(conn)
}
```

Note: The `Embedder::new()` is called each time `search_similar` is invoked. At query time in the MCP server, consider caching the Embedder instance in `ShireService` to avoid re-loading the model per query. This optimization can be done in Task 8 when integrating with the MCP tools.

**Step 2: Verify it compiles**

Run: `cargo check --features rag`
Expected: Compiles.

**Step 3: Commit**

```bash
git add src/rag/mod.rs
git commit -m "feat(rag): add public RAG API (embed, search, delete)"
```

---

### Task 7: Integrate embedding into the build pipeline

**Files:**
- Modify: `src/index/mod.rs`
- Modify: `src/db/mod.rs`

**Step 1: Add conditional RAG schema init to db/mod.rs**

In `src/db/mod.rs`, in the `open_or_create` function, after `create_schema(&conn)?;`:

```rust
#[cfg(feature = "rag")]
{
    // Attempt to init RAG storage; non-fatal if it fails
    if let Err(e) = crate::rag::init_storage(&conn) {
        eprintln!("[rag] Warning: failed to initialize RAG storage: {e}");
    }
}
```

**Step 2: Add embedding phase to build_index**

In `src/index/mod.rs`, in `build_index()`, after the "Phase 7+8: Extract symbols" block (after line ~1268) and before "Phase 9: Index files":

```rust
// Phase 8.5: Generate RAG embeddings (if enabled)
#[cfg(feature = "rag")]
if config.rag.enabled {
    let t = Instant::now();

    // Delete embeddings for removed packages
    for key in &diff.removed {
        // key is the manifest path — we need the package name
        // The package was already deleted from the DB, so embeddings
        // referencing those symbol IDs are orphaned.
        // A simpler approach: delete embeddings for symbol IDs that
        // no longer exist in the symbols table.
    }

    // Get all symbols that need embedding (from parsed/changed packages)
    let changed_packages: Vec<&str> = parsed_packages.iter().map(|(name, _, _)| name.as_str()).collect();

    if !changed_packages.is_empty() {
        // Delete old embeddings for changed packages
        for pkg in &changed_packages {
            let _ = crate::rag::delete_package_embeddings(&conn, pkg);
        }

        // Get the fresh symbols for these packages
        for pkg in &changed_packages {
            let symbols = db::queries::get_package_symbols(&conn, pkg, None)
                .unwrap_or_default();
            if symbols.is_empty() {
                continue;
            }
            let ids: Vec<i64> = conn
                .prepare("SELECT id FROM symbols WHERE package = ?1")
                .and_then(|mut s| {
                    s.query_map([pkg], |r| r.get(0))
                        .map(|rows| rows.filter_map(|r| r.ok()).collect())
                })
                .unwrap_or_default();

            if let Err(e) = crate::rag::embed_symbols(&conn, &symbols, &ids, &config.rag) {
                eprintln!("[rag] Warning: embedding failed for package '{}': {e}", pkg);
            }
        }
    }

    timings.push(("rag-embeddings", t.elapsed()));
}
```

Note: The exact integration depends on how `parsed_packages` is structured and what data is available at this point in the pipeline. The implementer should trace the types and adjust accordingly. The key principle is: for each changed package, delete old embeddings, fetch fresh symbols, embed them.

**Step 3: Run the full test suite**

Run: `cargo test`
Expected: All existing tests pass (no RAG feature = no changes).

Run: `cargo test --features rag`
Expected: All tests pass (RAG code compiles, unit tests for embedder/storage pass).

**Step 4: Commit**

```bash
git add src/index/mod.rs src/db/mod.rs
git commit -m "feat(rag): integrate embedding generation into build pipeline"
```

---

### Task 8: Integrate hybrid search into MCP tools

**Files:**
- Modify: `src/mcp/tools.rs`
- Modify: `src/mcp/mod.rs` (if ShireService needs config)
- Modify: `src/db/queries.rs` (add helper to fetch symbols by IDs)

**Step 1: Add a query to fetch symbols by rowid list**

In `src/db/queries.rs`, add:

```rust
/// Fetch symbols by their rowid (primary key ID).
/// Used by RAG hybrid search to look up vector search results.
pub fn get_symbols_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<SymbolRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
    let sql = format!(
        "SELECT name, kind, signature, package, file_path, line,
                visibility, parent_symbol, return_type, parameters
         FROM symbols WHERE id IN ({})",
        placeholders.join(",")
    );
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids.iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
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
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
}
```

**Step 2: Add RagConfig to ShireService**

The MCP server needs access to the RAG config for query-time embedding. Modify `ShireService` in `src/mcp/tools.rs`:

```rust
pub struct ShireService {
    pub(crate) conn: Mutex<Connection>,
    pub tool_router: ToolRouter<ShireService>,
    #[cfg(feature = "rag")]
    pub(crate) rag_config: Option<crate::config::RagConfig>,
}
```

Update the `new()` constructor to accept an optional RagConfig, and update `src/mcp/mod.rs` (where ShireService is created) to pass it through.

**Step 3: Implement hybrid search in search_symbols**

Modify the `search_symbols` method in `src/mcp/tools.rs`. After the existing FTS5 search, add:

```rust
// Hybrid search: merge vector results if RAG is available
#[cfg(feature = "rag")]
let results = {
    if let Some(ref rag_config) = self.rag_config {
        if rag_config.enabled {
            match crate::rag::search_similar(&conn, &params.query, 50, rag_config) {
                Ok(vec_results) if !vec_results.is_empty() => {
                    // RRF merge
                    merge_rrf(results, vec_results, &conn)
                }
                Ok(_) => results, // No vector results, keep FTS only
                Err(e) => {
                    eprintln!("[rag] Vector search failed, using FTS only: {e}");
                    results
                }
            }
        } else {
            results
        }
    } else {
        results
    }
};
```

**Step 4: Implement RRF merge function**

Add a helper function (can be in `tools.rs` or a separate module):

```rust
#[cfg(feature = "rag")]
fn merge_rrf(
    fts_results: Vec<SymbolRow>,
    vec_results: Vec<(i64, f64)>,
    conn: &Connection,
) -> Vec<SymbolRow> {
    use std::collections::HashMap;

    const K: f64 = 60.0;

    // Build RRF scores
    // Key: (name, package, file_path, line) as a unique symbol identifier
    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut symbol_map: HashMap<String, SymbolRow> = HashMap::new();

    // FTS results get their rank-based score
    for (rank, sym) in fts_results.iter().enumerate() {
        let key = format!("{}:{}:{}:{}", sym.name, sym.package, sym.file_path, sym.line);
        *scores.entry(key.clone()).or_default() += 1.0 / (K + rank as f64 + 1.0);
        symbol_map.entry(key).or_insert_with(|| sym.clone());
    }

    // Vector results get their rank-based score
    let vec_ids: Vec<i64> = vec_results.iter().map(|(id, _)| *id).collect();
    if let Ok(vec_symbols) = queries::get_symbols_by_ids(conn, &vec_ids) {
        for (rank, sym) in vec_symbols.iter().enumerate() {
            let key = format!("{}:{}:{}:{}", sym.name, sym.package, sym.file_path, sym.line);
            *scores.entry(key.clone()).or_default() += 1.0 / (K + rank as f64 + 1.0);
            symbol_map.entry(key).or_insert_with(|| sym.clone());
        }
    }

    // Sort by RRF score descending
    let mut ranked: Vec<(String, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Return top 50
    ranked.into_iter()
        .take(50)
        .filter_map(|(key, _)| symbol_map.remove(&key))
        .collect()
}
```

Note: `SymbolRow` needs to derive `Clone` for this to work. Add `#[derive(Clone)]` to `SymbolRow` in `queries.rs` if it doesn't already have it.

**Step 5: Verify compilation and tests**

Run: `cargo test --features rag`
Expected: All tests pass.

Run: `cargo test`
Expected: All tests pass without RAG feature.

**Step 6: Commit**

```bash
git add src/mcp/tools.rs src/mcp/mod.rs src/db/queries.rs
git commit -m "feat(rag): add hybrid FTS+vector search with RRF merging"
```

---

### Task 9: Integration test

**Files:**
- Modify: `tests/integration.rs` (optional, if RAG can be tested in CI)

This task is about verifying the end-to-end flow works. Since it requires model download, it should be an ignored test or a manual verification.

**Step 1: Write an ignored integration test**

Add to `tests/integration.rs`:

```rust
#[test]
#[ignore] // Requires --features rag and model download
fn test_build_with_rag_creates_embeddings() {
    // This test verifies the full pipeline:
    // 1. Create fixture monorepo with source files
    // 2. Create shire.toml with [rag] enabled = true
    // 3. Run shire build
    // 4. Open the DB and verify symbol_embeddings has data

    let dir = tempfile::TempDir::new().unwrap();
    create_fixture_monorepo(dir.path());

    // Write shire.toml with RAG enabled
    fs::File::create(dir.path().join("shire.toml"))
        .unwrap()
        .write_all(b"[rag]\nenabled = true\n")
        .unwrap();

    let bin = cargo_bin();
    let output = Command::new(&bin)
        .args(["build", "--root", dir.path().to_str().unwrap()])
        .output()
        .expect("failed to run shire build");

    assert!(output.status.success(), "build failed: {}", String::from_utf8_lossy(&output.stderr));

    // Verify embeddings exist
    let db_path = dir.path().join(".shire/index.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    // Note: need to load sqlite-vec extension to query the table
    // This may need adjustment based on how sqlite-vec is loaded
}
```

**Step 2: Manual verification**

Run against a real repo:

```bash
# Build with RAG
cargo build --features rag

# Create a test config
echo '[rag]
enabled = true' >> shire.toml

# Run build
./target/debug/shire build

# Run serve and test search via MCP
./target/debug/shire serve
```

**Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test(rag): add ignored integration test for RAG pipeline"
```

---

### Task 10: Watch daemon integration

**Files:**
- Modify: `src/watch/mod.rs`

The watch daemon already calls `index::build_index()` which now includes the RAG embedding phase. No code changes needed — the RAG integration in Task 7 is invoked automatically when the watch daemon triggers a rebuild.

**Step 1: Verify**

Read `src/watch/mod.rs` and confirm that `index::build_index()` is called with the full `Config` (which includes `rag` settings). It is — see line 185-189 where `build_config` is passed through.

**Step 2: No code changes needed**

The watch daemon calls `build_index(&build_root, &build_config, false, build_db.as_deref())` — the `build_config` already carries `config.rag`, so the embedding phase in Task 7 runs automatically.

**Step 3: Commit (if any docs or comments were added)**

No commit needed unless documentation was updated.
