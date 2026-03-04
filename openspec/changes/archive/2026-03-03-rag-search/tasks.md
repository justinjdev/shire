## 1. Dependencies and Feature Flag

- [x] 1.1 Add `fastembed`, `sqlite-vec`, and `zerocopy` as optional dependencies in `Cargo.toml` behind a `rag` feature flag
- [x] 1.2 Verify `sqlite-vec` compiles with rusqlite 0.36 (bundled) — if incompatible, find a working version combination

## 2. Configuration

- [x] 2.1 Add `RagConfig` struct to `src/config.rs` with `enabled`, `model`, and `cache_dir` fields, defaulting to disabled
- [x] 2.2 Wire `RagConfig` into `Config` with `#[serde(default)]` so missing `[rag]` section is valid

## 3. RAG Module — Storage

- [x] 3.1 Create `src/rag/mod.rs` with public API stubs and `#[cfg(feature = "rag")]` gating
- [x] 3.2 Create `src/rag/storage.rs` — `load_extension()` calling `sqlite3_auto_extension` with `sqlite3_vec_init`
- [x] 3.3 Implement `init_table(conn)` to create the `symbol_embeddings` vec0 virtual table (384-dim, cosine)
- [x] 3.4 Implement `insert_embeddings(conn, &[(i64, Vec<f32>)])` for batched vector inserts via zerocopy
- [x] 3.5 Implement `delete_embeddings_for_symbols(conn, &[i64])` to remove embeddings by symbol ID
- [x] 3.6 Implement `search_similar(conn, &[f32], limit) -> Vec<(i64, f64)>` for KNN cosine query

## 4. RAG Module — Embedder

- [x] 4.1 Create `src/rag/embedder.rs` — `Embedder::new(config)` wrapping `TextEmbedding::try_new()` with model selection
- [x] 4.2 Implement `symbol_to_text(symbol)` formatting: `{kind} {name} in {package} — {signature} @ {file_path}`
- [x] 4.3 Implement `embed_symbols(embedder, symbols) -> Vec<(i64, Vec<f32>)>` with batched embedding calls

## 5. Wire Storage into DB Layer

- [x] 5.1 Call `rag::storage::load_extension()` at process start (in `main.rs`) when compiled with `rag` feature
- [x] 5.2 Call `rag::storage::init_table()` from `db::open_or_create()` when RAG is enabled in config

## 6. Embed Symbols During Build

- [x] 6.1 Add embedding step in `index::build_index()` after symbol extraction (Phase 8.5): for each changed package, delete old embeddings and generate new ones
- [x] 6.2 Skip embedding step when RAG is disabled or embedder initialization fails (log and continue)

## 7. Hybrid Search in MCP

- [x] 7.1 Wire `RagConfig` into `ShireService` so `search_symbols` knows if RAG is available
- [x] 7.2 Add `get_symbols_by_ids(conn, &[i64])` query to `src/db/queries.rs` and derive `Clone` on `SymbolRow`
- [x] 7.3 Implement RRF merge in `search_symbols`: run FTS5 + vector search, compute `1/(60+rank)` scores, merge, sort descending, truncate to limit
- [x] 7.4 Fall back to FTS-only when RAG is disabled, embeddings table is empty, or vector search errors

## 8. Tests

- [x] 8.1 Unit test: `symbol_to_text` formatting with and without signature
- [x] 8.2 Unit test: RRF merge logic with overlapping and disjoint result sets
- [x] 8.3 Integration test: full build with `rag` feature → verify `symbol_embeddings` table is populated
- [x] 8.4 Integration test: hybrid search returns results for semantic query that FTS alone would miss
