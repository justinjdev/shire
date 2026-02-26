## Why

`build_index` in `src/index/mod.rs` is a ~350-line monolithic function with all 9 build phases inlined. This makes it hard to reason about individual phases, test them in isolation, and parallelize or instrument them (as required by the timing, parallel-extraction, and batch-db-operations changes). Extracting each phase into its own function is a prerequisite refactor that unblocks cleaner implementations of the performance changes.

## What Changes

- Extract each of the 9 build phases from `build_index` into standalone functions
- `build_index` becomes a thin orchestrator calling phase functions in sequence
- No behavioral changes — identical inputs produce identical outputs
- Helper functions (`upsert_package`, `upsert_symbols`, etc.) remain as-is or move into their respective phase functions

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `src/index/mod.rs` — `build_index` refactored from ~350 lines to ~50-line orchestrator with 9 phase functions
- No changes to public API, DB schema, CLI behavior, or test expectations
- Unblocks cleaner implementation of: `build-timing-instrumentation`, `parallel-symbol-extraction`, `batch-db-operations`
