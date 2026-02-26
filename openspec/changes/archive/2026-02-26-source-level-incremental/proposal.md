# Source-Level Incremental Indexing

## Why

Symbols go stale when source files change but manifests don't. Currently, symbol extraction (Phase 7) only runs for packages whose manifest was new or changed — tracked via the `parsed_packages` vec populated during Phase 3. If a developer adds a new exported function to a `.ts` file, renames a Go struct, or changes a Rust function signature, those changes are invisible to Shire until the next `--force` rebuild or an unrelated manifest edit.

This is a correctness issue. The index silently serves outdated symbol data with no indication that it's stale. For AI agents relying on Shire's MCP tools to locate symbols, stale data means wrong file:line references and missing definitions.

## What Changes

- **Source-level hashing**: compute an aggregate SHA-256 hash of all source files within each package directory (using the same walker and extension filters already in `symbols/walker.rs`)
- **New DB table**: `source_hashes` stores one hash per package, keyed by package name
- **Modified build pipeline**: after Phase 7 (symbol extraction for manifest-changed packages), add a new phase that checks source hashes for unchanged packages and re-extracts symbols when the source hash differs
- **Force and deletion cleanup**: `--force` clears `source_hashes`; package deletion removes the corresponding source hash row

## Capabilities

### Modified Capabilities
- `incremental-build`: source hash tracking — detect source file changes independently from manifest changes and trigger symbol re-extraction

## Impact

- `src/db/mod.rs` — new `source_hashes` table in schema
- `src/index/mod.rs` — new source hash computation, comparison, and conditional symbol re-extraction phase; `--force` clears source hashes; deletion removes source hash
- `src/index/hash.rs` — new `compute_source_hash()` function (aggregate hash of sorted source files)
- `src/symbols/walker.rs` — no changes (already provides the file list needed for hashing)
