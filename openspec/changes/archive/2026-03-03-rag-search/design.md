## Context

Shire indexes monorepos into SQLite with FTS5 full-text search across packages, symbols, and files. The MCP server exposes `search_symbols`, `search_packages`, and `search_files` tools. These work well for exact keyword queries but miss semantic matches — "find the auth middleware" won't match `verify_jwt_token`.

The goal is to augment `search_symbols` with vector similarity search so natural language queries return relevant results even without keyword overlap. Packages and files are well-served by keywords, so RAG targets symbols only.

## Goals / Non-Goals

**Goals:**
- Semantic search for symbols via vector embeddings
- Fully offline after first model download (no API keys)
- Opt-in via config — zero impact on users who don't enable it
- Incremental embedding updates aligned with existing source hash tracking
- Non-fatal failures — RAG errors never break the core index or search

**Non-Goals:**
- Embedding packages or files (keywords suffice)
- Replacing FTS5 (vectors augment it)
- Supporting multiple embedding backends or API-based providers
- Chunk-level or line-level embeddings (symbol granularity is sufficient)

## Decisions

### 1. Libraries: fastembed + sqlite-vec

**fastembed** v5 (wraps ONNX Runtime): High-level sync embedding API. Handles tokenization, batching, model download and caching. 25+ preconfigured models via `EmbeddingModel` enum.

**sqlite-vec** v0.1 (SQLite extension): Vector storage and KNN search in the same SQLite database. Supports `float[N]` columns with `distance_metric=cosine`. Brute-force search, fine at symbol scale (<100k vectors).

Both are optional dependencies behind a `rag` Cargo feature flag:

```toml
[features]
default = []
rag = ["dep:fastembed", "dep:sqlite-vec", "dep:zerocopy"]

[dependencies]
fastembed = { version = "5", optional = true }
sqlite-vec = { version = "0.1", optional = true }
zerocopy = { version = "0.7", optional = true }
```

`zerocopy` is needed to pass `Vec<f32>` as bytes to sqlite-vec without copying.

**Why not ort directly?** fastembed wraps ort with tokenization, model download, and batching. Using ort directly means reimplementing all of that.

**Why not candle?** Pure Rust but requires manual tokenizer wiring and has no model download pipeline.

### 2. Embedding text representation

Each symbol is serialized into a single text string for embedding:

```
{kind} {name} in {package} — {signature} @ {file_path}
```

Example:
```
function authenticate in auth-service — fn authenticate(req: Request, key: ApiKey) -> Result<Token> @ src/auth/middleware.rs
```

If signature is null, omit it: `struct UserConfig in shared-types @ src/types.ts`

### 3. Model: BAAI/bge-small-en-v1.5

- `EmbeddingModel::BGESmallENV15` in fastembed
- 384 dimensions (~1.5KB per symbol vector)
- ~33MB ONNX model, auto-downloaded on first use
- Fast inference (~1000 embeddings/sec on M1)

### 4. Storage via sqlite-vec vec0

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS symbol_embeddings USING vec0(
    symbol_id INTEGER PRIMARY KEY,
    embedding float[384] distance_metric=cosine
);
```

Loading the extension into rusqlite:

```rust
use sqlite_vec::sqlite3_vec_init;
use rusqlite::ffi::sqlite3_auto_extension;

unsafe {
    sqlite3_auto_extension(Some(std::mem::transmute(
        sqlite3_vec_init as *const ()
    )));
}
```

This must be called once before opening any connection. All subsequent connections get vec0 support automatically.

Inserting vectors (binary format via zerocopy):

```rust
use zerocopy::AsBytes;
let embedding: Vec<f32> = /* from fastembed */;
stmt.execute(params![symbol_id, embedding.as_bytes()])?;
```

KNN query:

```sql
SELECT symbol_id, distance
FROM symbol_embeddings
WHERE embedding MATCH ?1
ORDER BY distance
LIMIT ?2
```

The query vector is also passed as `.as_bytes()`.

### 5. Hybrid search with Reciprocal Rank Fusion (RRF)

When RAG is enabled and embeddings exist, `search_symbols` runs two searches:

1. **FTS5 path**: Existing keyword search, returns ranked results
2. **Vector path**: Embed the query, cosine similarity via sqlite-vec, top-K results

Merged using RRF:

```
rrf_score(d) = 1/(k + rank_fts(d)) + 1/(k + rank_vec(d))
```

Where `k=60`. Documents in only one list get only that term. Sorted by `rrf_score` descending, truncated to result limit.

**Why RRF?** FTS5 BM25 scores and cosine distances are on different scales. RRF works on ranks, not scores, so no normalization needed.

### 6. Incremental embedding updates

Piggyback on existing incremental build:

- Package source hash changes → symbols re-extracted → old embeddings deleted, new ones generated
- Package removed → embeddings deleted
- Package unchanged → embeddings untouched

No separate hash tracking — if symbols changed, re-embed.

### 7. Configuration

```toml
[rag]
enabled = true
# model = "BAAI/bge-small-en-v1.5"  # optional override
# cache_dir = "~/.cache/shire/models"  # optional override
```

### 8. Module structure

```
src/rag/
  mod.rs        — public API: init_storage(), embed_symbols(), search_similar(), is_available()
  embedder.rs   — fastembed wrapper: model init, symbol_to_text(), batched embedding
  storage.rs    — sqlite-vec: extension loading, table creation, vector CRUD, KNN query
```

All behind `#[cfg(feature = "rag")]`.

### 9. Error handling: all RAG failures are non-fatal

- Model download fails → log error, skip embeddings, build succeeds
- Individual symbol embedding fails → log warning, skip that symbol, continue
- sqlite-vec extension load fails → log error, disable vector search
- Vector search fails at query time → log warning, return FTS-only results
- Embeddings table empty → silently fall back to FTS-only

### 10. sqlite-vec extension loading strategy

The `sqlite3_auto_extension` call registers the extension globally for all connections. This needs to happen once at process start, before any `Connection::open` call. The `rag` module exposes an `init()` function that handles this, called from `main()` when compiled with the `rag` feature.

For the read-only MCP server connection (`open_readonly`), the extension also needs to be loaded to support KNN queries at search time.

## Risks / Trade-offs

- **Binary size**: +30-50MB with ONNX Runtime. → Cargo feature flag keeps default binary small.
- **First-run network**: Model download needs internet. → Clear error message, cached after first download.
- **sqlite-vec pre-v1**: API may change. → All usage isolated in `src/rag/storage.rs`.
- **fastembed pins ort RC**: `ort =2.0.0-rc.x`. → fastembed's problem to manage.
- **Embedding quality for code**: BGE-small trained on natural language. → Symbol names/signatures are semi-natural-language. Swap model via config if needed.
- **rusqlite version**: Shire uses rusqlite 0.36, sqlite-vec examples show 0.31. → Need to verify compatibility at implementation time. The `sqlite3_auto_extension` FFI interface is stable across versions.
