## Why

Shire's FTS5 search works well for keyword-exact queries but fails on semantic intent. An LLM asking "find the authentication middleware" won't match a symbol named `verify_jwt_token` because no keywords overlap. Monorepos have enough symbols that keyword-miss queries are common — the gap between what users mean and what they type is the core problem.

## What Changes

- Add optional RAG (vector similarity) search alongside existing FTS5 keyword search
- Embed symbols at index time using a local embedding model (no API keys, fully offline after first model download)
- Store embeddings in the existing SQLite DB via the `sqlite-vec` extension
- Merge FTS5 and vector results using Reciprocal Rank Fusion (RRF) for hybrid search
- All RAG functionality is opt-in via `[rag]` config section and `rag` Cargo feature flag

## Capabilities

### New Capabilities
- `rag-embedding`: Generate vector embeddings for symbols using a local embedding model (fastembed + ONNX Runtime), stored in SQLite via sqlite-vec
- `rag-search`: Cosine similarity search over symbol embeddings, merged with FTS5 results via RRF ranking

### Modified Capabilities
- `symbol-querying`: `search_symbols` MCP tool performs hybrid FTS5 + vector search when RAG is enabled, falling back to FTS-only when disabled or embeddings are unavailable
- `configuration`: New `[rag]` section in `shire.toml` with `enabled` flag and optional model/cache settings
- `incremental-build`: Embedding generation integrated into the build pipeline, incremental via existing source hash tracking

## Impact

- **`Cargo.toml`**: New optional dependencies `fastembed` and `sqlite-vec` behind `rag` feature flag
- **`src/rag/`** (new): Embedding generation, vector storage, and hybrid search logic
- **`src/db/mod.rs`**: Conditionally create `symbol_embeddings` virtual table
- **`src/index/mod.rs`**: Post-symbol-extraction embedding step when RAG is enabled
- **`src/mcp/tools.rs`**: `search_symbols` merges vector results with FTS5 results
- **`src/config.rs`**: Parse `[rag]` config section
- **`src/watch/mod.rs`**: Re-embed changed symbols during incremental rebuilds
- **Binary size**: +30-50MB when compiled with `--features rag` (ONNX Runtime)
