## Tasks

### 1. Add RAG dependencies and feature flag
- Add `fastembed` and `sqlite-vec` as optional dependencies in `Cargo.toml` behind `rag` feature
- Verify the crate compiles with and without the feature flag

### 2. Add `[rag]` config parsing
- Add `RagConfig` struct to `src/config.rs` with `enabled`, `model`, `cache_dir` fields
- Parse `[rag]` section from `shire.toml`, defaulting to disabled
- Unit test: config with RAG enabled, config without RAG section

### 3. Create `src/rag/embedder.rs`
- Initialize fastembed model (lazy, on first use)
- Function to convert a symbol to its text representation
- Function to embed a batch of text strings, returning vectors
- Handle model download with progress messaging
- Handle download/inference failures gracefully (log + continue)
- Unit test: text representation format

### 4. Create `src/rag/storage.rs`
- Load sqlite-vec extension into a connection
- Create `symbol_embeddings` vec0 virtual table
- Functions: insert embeddings, delete embeddings by symbol IDs, query by vector similarity
- Unit test: round-trip insert and cosine query

### 5. Create `src/rag/mod.rs` public API
- `embed_symbols(conn, symbols, config)` — orchestrates embedding generation and storage
- `search_similar(conn, query_text, limit)` — embeds query, runs vector search, returns ranked symbol IDs
- `is_available(conn)` — checks if embeddings table exists and has data
- All functions behind `#[cfg(feature = "rag")]`

### 6. Integrate embedding into build pipeline
- In `src/index/mod.rs`, after symbol extraction, call `rag::embed_symbols()` if RAG is enabled
- Delete stale embeddings when packages are removed or updated
- Incremental: only re-embed packages whose source hash changed
- Integration test: build with RAG enabled, verify embeddings exist

### 7. Integrate hybrid search into MCP tools
- In `src/mcp/tools.rs`, modify `search_symbols` to run vector search alongside FTS5 when RAG is available
- Implement RRF merging of the two result sets
- Fall back to FTS-only on any vector search error
- Integration test: semantic query returns relevant symbols

### 8. Integrate with watch daemon
- In `src/watch/mod.rs`, include re-embedding in the incremental rebuild path
- Only when RAG is enabled in config

### 9. Conditionally create schema
- In `src/db/mod.rs`, conditionally create the `symbol_embeddings` table when `rag` feature is enabled
- Ensure schema creation is idempotent
