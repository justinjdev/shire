# Autooptimize Session Summary — 2026-03-27

Branch: `opt/autooptimize-20260327` (11 commits, +313 / -252 lines across 4 files)

## Build Performance

**Benchmark repo:** turborepo (~900MB, 400 packages, 8000 symbols, 5450 files)

**Result: 549ms → 485ms (12% faster)**

### Experiments Run (17 total)

| # | Experiment | Module | Delta | Kept | Notes |
|---|-----------|--------|-------|------|-------|
| 1 | BLAKE3 hashing | index | +0.3% | no | Not faster than SHA-256 at these file sizes |
| 2 | page_size=8192 + EXCLUSIVE lock | db | +1.0% | no | No measurable gain |
| 3 | **FTS triggers once per phase** | index | **-3.2%** | **yes** | Was dropping/recreating triggers 400x per package |
| 4 | **Bulk FTS rebuild** | index | **-5.4%** | **yes** | Single `rebuild` instead of 400 per-package FTS syncs |
| 5 | Batch size 500 (vs 100) | index | -1.3% | no | Within noise |
| 6 | Remove sort from walk_source_files | symbols | +7.5% | no | Broke aggregate hash determinism |
| 7 | Bulk DELETE IN() for symbols | index | +1.3% | no | No measurable gain |
| 8 | WAL autocheckpoint=0 | db | +3.0% | no | Manual checkpoint added overhead |
| 9 | synchronous=OFF | db | +3.4% | no | Extra PRAGMA calls added overhead |
| 10 | **Skip FTS optimize on fresh builds** | db | **-1.0%** | **yes** | Also reduced DB size by 20% |
| 11 | Drop files_fts triggers | db | +1.4% | no | Rebuild overhead exceeded trigger savings |
| 12 | prepare_cached for symbols | db | +0.4% | no | Within noise |
| 13 | Pre-compute extensions list | index | -0.2% | no | Marginal, within noise |
| 14 | Drop B-tree indexes during build | db | +11.9% | no | Index recreation cost dominated |
| 15 | thread_local Parser reuse | symbols | +4.7% | no | RefCell overhead exceeded savings at this scale |
| 16 | **Prepared statement loop (symbols)** | index | **-2.8%** | **yes** | Eliminated 80k Box<dyn ToSql> heap allocs |
| 17 | **Prepared statement loop (hashes)** | index | **+0.2%** | **yes** | Same perf, -51 lines — simpler code |

### Key Learnings

- **Per-package SQLite overhead** was the main source of waste (experiments 3, 4, 10)
- Many "textbook" optimizations (BLAKE3, larger page sizes, dropping indexes, thread-local parsers) actually regressed at this benchmark scale due to fixed overhead exceeding savings
- Web-researched recommendations (via parallel sub-agents) were hit-or-miss: drop indexes (+11.9% worse), thread-local parser (+4.7% worse), prepared statement loop (-2.8% better)
- The turborepo benchmark at 900MB / 5450 files is near the floor for meaningful optimization — a 3-4GB polyglot monorepo would better surface wins

## RAG Overhaul

### File-Level Embeddings (replacing symbol-level)

Switched from embedding individual symbols (~8000 at turborepo scale) to embedding source files (~905 files with symbols). Each file embedding is constructed from its aggregated symbol names and kinds.

**Advantages:**
- Richer semantic context per embedding (whole file vs single function signature)
- Stable file IDs — no churn when symbols are re-extracted with new autoincrement IDs
- Natural incremental updates: only embed files missing from the embeddings table
- Fewer embeddings: 905 source files vs 8000+ symbols (for turborepo)

### Background Embedding

Embedding inference runs in a background thread after the build completes. The build summary prints immediately and FTS search is available right away. RAG search becomes available when embedding finishes.

| Scenario | Before (blocking) | After (background) |
|----------|-------------------|-------------------|
| Shire full build (234 files) | 2.8s | 0.1s build + 0.8s background |
| Turborepo full build (5451 files) | 26s+ blocking | ~0.5s build + 56s background |
| Incremental build (no changes) | 26ms | 29ms (no RAG overhead) |

### Source-Only Filter

Only embeds files that have extracted symbols (source code files). Skips configs, docs, READMEs etc. that produce poor embeddings. Reduced embedding count from 5450 → 905 on turborepo (83% reduction).

### Semantic Search Quality (turborepo)

| Query (semantic) | FTS Results | RAG Results |
|-----------------|-------------|-------------|
| "task scheduling" | 0 | 20 (Task, Planned, Running, Finished) |
| "cache invalidation" | 0 | 20 (revalidate, turborepo-cache) |
| "file watching" | 0 | 20 (HashWatcher, InputGlobs, filewatch) |
| "incremental rebuild" | 0 | 7 (send_rebuild, RebuildMessage) |
| "database write optimization" | 0 | 20 |

## Commits

```
df84fd8 perf: run RAG embedding in background thread
9c0d17a perf: only embed source files with symbols, not all files
514fa71 feat: switch RAG from symbol-level to file-level embeddings
31ed789 perf: optimize RAG embedding path
f740fa4 experiment: same prepared statement optimization for file_hashes and source_hashes
d1a7443 experiment: replace multi-row INSERT with prepared statement loop
4d6276b experiment: skip FTS optimize on full builds since FTS was just rebuilt
0771b84 cleanup: remove unused upsert_symbols function
09b658d fix: use FTS5 delete-all + rebuild commands for content-sync tables
35e83e4 experiment: bulk FTS rebuild instead of per-package FTS sync
099bacd experiment: drop/recreate FTS triggers once per build phase instead of per-package
```

## Files Changed

- `src/index/mod.rs` — Build pipeline: FTS trigger optimization, bulk FTS rebuild, prepared statements, background RAG thread
- `src/mcp/tools.rs` — Hybrid search: file-level vector search → symbol lookup, improved RAG init logging
- `src/rag/embedder.rs` — Added `FileForEmbedding`, `file_to_text`, `embed_files` for file-level RAG
- `src/rag/storage.rs` — Added `file_embeddings` table, `insert_file_embeddings`, `search_similar_files`
