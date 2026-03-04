## Why

Shire's FTS5 search works well for keyword-exact queries but fails on semantic intent. An LLM asking "find the authentication middleware" won't match a symbol named `verify_jwt_token` because no keywords overlap. Monorepos have enough symbols that keyword-miss queries are common — the gap between what users mean and what they type is the core problem.

## What Changes

- Add optional vector similarity search alongside existing FTS5 keyword search
- Embed symbols at index time using a local embedding model (fastembed + ONNX Runtime, no API keys, fully offline after first model download)
- Store embeddings in the existing SQLite DB via the `sqlite-vec` extension
- Merge FTS5 and vector results using Reciprocal Rank Fusion (RRF) for hybrid search in `search_symbols`
- All RAG functionality is opt-in via `[rag]` config section and `rag` Cargo feature flag
- Binary size increases ~30-50MB when compiled with the `rag` feature (ONNX Runtime)

## Capabilities

### New Capabilities
- `rag-embedding`: Generate and store vector embeddings for symbols using a local embedding model, with incremental updates tied to the existing source hash system
- `rag-search`: Cosine similarity search over symbol embeddings, merged with FTS5 results via Reciprocal Rank Fusion ranking

### Modified Capabilities
- `symbol-querying`: `search_symbols` performs hybrid FTS5 + vector search when RAG is enabled, falling back to FTS-only when disabled or embeddings are unavailable
- `configuration`: New `[rag]` section in `shire.toml` with `enabled`, `model`, and `cache_dir` fields

## Impact

- **`Cargo.toml`**: New optional dependencies `fastembed` and `sqlite-vec` behind `rag` feature flag
- **`src/rag/`** (new module): Embedding generation (`embedder.rs`), vector storage (`storage.rs`), public API (`mod.rs`)
- **`src/db/mod.rs`**: Conditionally create `symbol_embeddings` virtual table via sqlite-vec
- **`src/index/mod.rs`**: Post-symbol-extraction embedding step when RAG is enabled
- **`src/mcp/tools.rs`**: `search_symbols` merges vector results with FTS5 results
- **`src/config.rs`**: Parse `[rag]` config section
- **Watch daemon**: No code changes needed — `build_index` already receives full config, so RAG embedding runs automatically during incremental rebuilds
