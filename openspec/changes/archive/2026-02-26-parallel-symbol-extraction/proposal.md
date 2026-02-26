## Why

Symbol extraction (Phases 7 and 8) processes packages sequentially in a `for` loop. Each package's source files are walked, parsed with tree-sitter, and inserted into the DB one at a time. This is embarrassingly parallel — packages are independent — and is likely the single largest bottleneck on full builds of large monorepos.

## What Changes

- Add `rayon` dependency for data parallelism
- Parallelize symbol extraction across packages in Phase 7 (new/changed packages) and Phase 8 (source-level incremental)
- Collect symbols in parallel, then batch-insert into SQLite on the main thread (SQLite is single-writer)
- Source hash computation in Phase 8 also parallelized across unchanged packages

## Capabilities

### New Capabilities
- `parallel-extraction`: Parallel symbol extraction and source hashing across packages using rayon

### Modified Capabilities

## Impact

- `Cargo.toml` — New `rayon` dependency
- `src/index/mod.rs` — Phases 7 and 8 refactored to use `par_iter` with collected results
- `src/symbols/mod.rs` — Must be thread-safe (already is — no shared mutable state)
- `src/index/hash.rs` — Must be thread-safe (already is — pure functions)
- Speedup scales with number of packages and available cores
