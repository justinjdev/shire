## Context

Shire indexes monorepos into SQLite with FTS5 full-text search across packages, symbols, and files. The MCP server exposes `search_symbols`, `search_packages`, and `search_files` tools. These work well for exact keyword queries but miss semantic matches — "find the auth middleware" won't match `verify_jwt_token`.

The goal is to augment `search_symbols` with vector similarity search so natural language queries return relevant results even without keyword overlap. Packages and files are well-served by keywords (names and paths are already descriptive), so RAG targets symbols only.

## Goals / Non-Goals

**Goals:**
- Semantic search for symbols via vector embeddings
- Fully offline after first model download (no API keys)
- Opt-in via config — zero impact on users who don't enable it
- Incremental embedding updates aligned with existing source hash tracking
- Non-fatal failures — RAG errors never break the core index or search

**Non-Goals:**
- Embedding packages or files (keywords suffice for these)
- Replacing FTS5 (it remains the primary search path; vectors augment it)
- Supporting multiple embedding backends or API-based providers (can add later)
- Chunk-level or line-level embeddings (symbol granularity is sufficient)

## Decisions

### 1. Libraries: fastembed + sqlite-vec

**fastembed** (v5, wraps ONNX Runtime): High-level embedding API with 25+ preconfigured models. Handles tokenization, batching, and model management. Sync API, no Tokio needed for the embedding step.

**sqlite-vec** (v0.1): SQLite extension for vector storage and cosine similarity search. Keeps embeddings in the same database file. Brute-force search, which is fine at symbol scale (tens of thousands, not millions).

Both are optional dependencies behind a `rag` Cargo feature flag:

```toml
[features]
rag = ["dep:fastembed", "dep:sqlite-vec"]

[dependencies]
fastembed = { version = "5", optional = true }
sqlite-vec = { version = "0.1", optional = true }
```

**Why not ort directly?** fastembed wraps ort with tokenization, model download, and batching. Using ort directly means reimplementing all of that.

**Why not candle?** Pure Rust but requires manual tokenizer wiring and has no model download pipeline. More code for no meaningful benefit.

### 2. Embedding text representation

Each symbol is serialized into a single text string for embedding:

```
{kind} {name} in {package} — {signature} @ {file_path}
```

Example:
```
function authenticate in auth-service — fn authenticate(req: Request, key: ApiKey) -> Result<Token> @ src/auth/middleware.rs
```

This captures:
- **Semantic role** from kind (function, class, struct, trait)
- **Purpose** from name (naming conventions carry meaning)
- **Type information** from signature (parameters, return types)
- **Location context** from package and file path

### 3. Model choice: BAAI/bge-small-en-v1.5

- 384 dimensions (compact vectors, ~1.5KB per symbol)
- ~33MB ONNX model, auto-downloaded by fastembed on first use
- Good quality for code-related text despite being trained on natural language
- Fast inference (~1000 embeddings/sec on M1)

### 4. Storage via sqlite-vec

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS symbol_embeddings USING vec0(
    symbol_id INTEGER PRIMARY KEY,
    embedding FLOAT[384]
);
```

Lives in the same SQLite database. Triggers are not supported for vec0 tables, so embedding inserts/deletes are managed explicitly in the build pipeline (not via triggers like FTS5).

### 5. Hybrid search with Reciprocal Rank Fusion (RRF)

When RAG is enabled and embeddings exist, `search_symbols` runs two parallel searches:

1. **FTS5 path**: Existing keyword search, returns ranked results
2. **Vector path**: Embed the query string, cosine similarity search against `symbol_embeddings`, returns top-K results

Results are merged using RRF:

```
rrf_score(d) = 1/(k + rank_fts(d)) + 1/(k + rank_vec(d))
```

Where `k=60` (standard constant). Documents appearing in only one result set get only that term. The merged list is sorted by `rrf_score` descending and truncated to the result limit.

**Why RRF over score normalization?** FTS5 BM25 scores and cosine similarities are on different scales. RRF works on ranks, not scores, so no normalization is needed. It's simple, well-understood, and empirically effective.

### 6. Incremental embedding updates

Embeddings piggyback on the existing incremental build system:

- When a package's source hash changes, its symbols are re-extracted (current behavior)
- After re-extraction, old embeddings for that package are deleted and new ones generated
- When a package is removed, its embeddings are deleted
- Packages with unchanged source hashes skip embedding entirely

No separate hash tracking for embeddings — if symbols changed, re-embed.

### 7. Configuration

```toml
[rag]
enabled = true
# model = "BAAI/bge-small-en-v1.5"  # optional override
# cache_dir = "~/.cache/shire/models"  # optional override
```

Minimal config surface. The model and cache_dir have sensible defaults. Most users just set `enabled = true`.

### 8. Module structure

```
src/rag/
  mod.rs        — public API: embed_symbols(), search_similar(), is_available()
  embedder.rs   — fastembed wrapper: model init, text→vector, batched processing
  storage.rs    — sqlite-vec: table creation, vector insert/delete/query
```

All behind `#[cfg(feature = "rag")]`. The integration points use conditional compilation:

```rust
#[cfg(feature = "rag")]
if config.rag.enabled {
    rag::embed_symbols(&conn, &symbols)?;
}
```

### 9. Error handling: all RAG failures are non-fatal

- **Model download fails**: Log error with message, skip embeddings, build succeeds
- **Individual symbol embedding fails**: Log warning, skip that symbol, continue
- **sqlite-vec extension load fails**: Log error, disable vector search for this session
- **Vector search fails at query time**: Log warning, return FTS-only results
- **Embeddings table empty**: Silently fall back to FTS-only (user may not have built with RAG yet)

The index always builds. Embeddings are best-effort.

### 10. Build UX

First run with RAG enabled:
```
$ shire build
Building index...
  Downloading embedding model (BAAI/bge-small-en-v1.5, ~33MB)... done
  Generating embeddings for 1,247 symbols... done (2.3s)
Index built: 42 packages, 1,247 symbols, 3,891 files
```

Subsequent runs:
```
$ shire build
Building index...
  Updated 3 packages (incremental)
  Re-embedding 47 symbols... done (0.1s)
Index built: 42 packages, 1,250 symbols, 3,895 files
```

## Risks / Trade-offs

- **Binary size**: +30-50MB with ONNX Runtime statically linked. → Mitigation: Cargo feature flag, so users who don't need RAG get the current binary. Homebrew can offer both variants.
- **First-run network requirement**: Model download needs internet. → Mitigation: Clear error message with manual download instructions. Model is cached after first download.
- **sqlite-vec is pre-v1**: API may change. → Mitigation: All sqlite-vec usage is isolated in `src/rag/storage.rs`. If the API changes, only one file needs updating.
- **fastembed pins ort RC**: `fastembed` depends on `ort =2.0.0-rc.11`. → Mitigation: This is fastembed's problem to manage. Track their releases.
- **Embedding quality for code**: bge-small is trained on natural language, not code. → Acceptable for symbol names and signatures which are semi-natural-language. If quality is insufficient, swapping to a code-specific model is a config change.
- **Brute-force vector search**: sqlite-vec does linear scan. → Fine at symbol scale (<100k). If shire indexes million-symbol repos, can add IVF indexing later.
