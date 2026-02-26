# Parallel Symbol Extraction — Design

## Context

The `build_index` function in `src/index/mod.rs` processes symbol extraction in two sequential phases:

- **Phase 7** iterates over `parsed_packages` (new/changed packages from manifest diff). For each package, it calls `extract_symbols_for_package` and `compute_source_hash`, then writes results to SQLite via `upsert_symbols` and `upsert_source_hash`.
- **Phase 8** iterates over `diff.unchanged` manifests. For each, it queries the DB for package info and stored source hash, computes the current source hash, and if the hash differs, calls `extract_symbols_for_package` again.

Both phases are sequential `for` loops. Each iteration is independent: different package, different source files, separate tree-sitter parser instances created per-call. The CPU-bound work (tree-sitter parsing, SHA-256 hashing, file walking) dominates wall-clock time on large monorepos with many packages. SQLite is single-writer, so the DB inserts cannot be parallelized.

## Goals / Non-Goals

**Goals:**
- Parallelize symbol extraction and source hash computation across packages using rayon
- Maintain correctness: the final database state must be identical to sequential execution
- Scale extraction throughput with available CPU cores

**Non-Goals:**
- Parallel DB writes — SQLite is single-writer; inserts remain sequential on the main thread
- Parallel manifest parsing (Phase 3) — already fast, not the bottleneck
- Custom thread pool tuning — rayon's defaults (one thread per core) are appropriate
- Parallel file walking within a single package — packages are the unit of parallelism

## Decisions

### 1. Use `rayon::par_iter` for extraction, collect results, insert sequentially

Parallelize the CPU/IO-bound work (tree-sitter parsing, file reading, SHA-256 computation) using `rayon::par_iter`. Each package produces a result tuple containing its symbols and source hash. Results are collected into a `Vec`, then inserted into SQLite sequentially on the main thread.

```rust
// Phase 7 sketch
let results: Vec<_> = parsed_packages
    .par_iter()
    .map(|(name, path, kind)| {
        let syms = symbols::extract_symbols_for_package(repo_root, path, kind);
        let hash = hash::compute_source_hash(repo_root, path, kind);
        (name, syms, hash)
    })
    .collect();

for (name, syms, hash) in &results {
    // sequential DB writes
}
```

**Why:** SQLite cannot handle concurrent writes, so the parallelism boundary is the compute work. Collecting into a Vec is the simplest way to bridge parallel computation with sequential insertion. The memory cost of holding all symbols in flight simultaneously is acceptable — even large monorepos produce symbol sets that fit comfortably in memory.

### 2. Phase 7 and Phase 8 parallelized independently

Both phases get their own `par_iter` block. They are not merged into a single parallel pass because:

- Phase 7 operates on `parsed_packages` (new/changed), Phase 8 on `diff.unchanged` — different input sets.
- Phase 8 requires a DB query per package to fetch stored source hashes. These reads must happen before the parallel compute. The approach is: query all stored hashes into a HashMap first, then `par_iter` over unchanged packages to compute current hashes and conditionally extract symbols.

For Phase 8, the DB reads are batched upfront:

```rust
// Pre-fetch all package info and stored hashes for unchanged manifests
let unchanged_pkgs: Vec<_> = diff.unchanged.iter().filter_map(|manifest| {
    // DB query for (name, kind, stored_hash)
}).collect();

// Parallel compute
let results: Vec<_> = unchanged_pkgs
    .par_iter()
    .filter_map(|(name, path, kind, stored_hash)| {
        let current_hash = hash::compute_source_hash(...)?;
        if stored_hash.as_deref() != Some(&current_hash) {
            let syms = symbols::extract_symbols_for_package(...);
            Some((name, syms, current_hash))
        } else {
            None
        }
    })
    .collect();
```

### 3. No changes needed in the symbols or hash modules

`extract_symbols_for_package` is already a pure function: it takes a repo root, package path, and kind, walks source files, creates a fresh tree-sitter parser per file, and returns `Vec<SymbolInfo>`. No shared mutable state.

`compute_source_hash` is similarly pure: file reads and SHA-256 computation with no shared state.

Both functions are `Send + Sync` compatible as-is. No refactoring, no `Arc`, no `Mutex`.

### 4. Rayon over alternatives

| Option | Verdict |
|---|---|
| `rayon::par_iter` | **Chosen.** Standard data-parallelism crate for Rust. Work-stealing scheduler, zero-config thread pool, `par_iter` is a drop-in for `iter`. |
| `tokio` async tasks | Overkill. This is CPU-bound work with synchronous file IO. Async adds complexity (runtime, `Send` bounds, `spawn_blocking`) with no benefit. |
| `crossbeam` scoped threads | More manual. Requires explicit thread management. Rayon's `par_iter` is higher-level and handles work distribution automatically. |
| `std::thread::spawn` | No work-stealing, manual join handling, no automatic core-count scaling. |

## Risks / Trade-offs

**Non-deterministic result ordering.** `par_iter` does not preserve iteration order. Mitigated by the fact that `upsert_symbols` and `upsert_source_hash` are keyed operations (INSERT OR REPLACE) — insertion order does not affect final DB state. If deterministic insertion order is desired for debugging, results can be sorted by package name before the sequential insert loop.

**Increased peak memory.** Sequential execution processes and inserts one package at a time. Parallel execution collects all symbols across all packages before inserting any. For a monorepo with 100 packages averaging 200 symbols each, this is ~20K `SymbolInfo` structs in flight — well within acceptable memory bounds.

**New dependency: `rayon`.** Adds a compile-time dependency. Rayon is one of the most widely-used Rust crates (100M+ downloads), maintained by the Rust project ecosystem, and has a stable API. The risk of supply-chain or maintenance issues is minimal.

**Phase 8 DB read batch.** Pre-fetching all stored hashes for unchanged packages into memory changes the access pattern from streaming (one query per iteration) to batch (all queries upfront). This is actually more efficient for SQLite (fewer statement preparations) and the memory cost is negligible (one hash string per package).
