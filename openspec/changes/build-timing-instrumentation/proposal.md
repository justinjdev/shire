## Why

The build pipeline has zero timing instrumentation. There are no `Instant`/`Duration` measurements anywhere in `build_index`, making it impossible to identify bottlenecks or validate that performance improvements actually help. Before optimizing anything, we need to measure.

## What Changes

- Add per-phase timing to `build_index` using `std::time::Instant` for each of the 9 phases
- Print a timing breakdown to stderr after the build summary (e.g., `Phase 1 (walk): 12ms, Phase 3 (parse): 45ms, ...`)
- Store total build duration in `shire_meta` for tracking over time

## Capabilities

### New Capabilities
- `build-timing`: Per-phase timing instrumentation for the build pipeline with stderr output and metadata storage

### Modified Capabilities

## Impact

- `src/index/mod.rs` — `build_index` function gains timing wrappers around each phase
- `src/db/queries.rs` — New `shire_meta` key for build duration
- No new dependencies (uses `std::time`)
- No behavioral changes to build output on stdout — timing goes to stderr
