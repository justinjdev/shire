# Build Timing Instrumentation — Design

## Context

The `build_index` function in `src/index/mod.rs` executes 9 sequential phases: walk, workspace context, diff, parse, remove deleted, recompute internals, update hashes, extract symbols (+ source-level re-extraction), and index files. There is currently no timing instrumentation anywhere in the pipeline. Without measurements, it is impossible to identify bottlenecks or validate that performance changes have any effect.

## Goals / Non-Goals

**Goals:**
- Measure wall-clock duration of each of the 9 build phases using `std::time::Instant`/`Duration`
- Print a timing breakdown to stderr after the build completes
- Store total build duration in `shire_meta` as `total_duration_ms`

**Non-Goals:**
- Not a profiler — no CPU/memory tracking, no flamegraphs
- No tracing spans or structured logging (no `tracing` crate)
- No external dependencies — only `std::time`
- No per-file or per-package timing within a phase
- No configurable verbosity — timing always prints

## Decisions

### 1. Timing structure: Instant per phase, collect into Vec

Record `Instant::now()` before and after each phase. Store results as `Vec<(&str, Duration)>` where the string is a human-readable phase label. Print the full list at the end of the build.

```rust
let t0 = Instant::now();
// ... phase work ...
timings.push(("walk", t0.elapsed()));
```

This is the simplest approach. No structs, no traits, no abstraction — just a local Vec that accumulates timing pairs. The phase labels match the existing comments in `build_index` (e.g., "walk", "workspace-context", "diff", "parse", "remove-deleted", "recompute-internals", "update-hashes", "extract-symbols", "index-files").

### 2. Output to stderr, not stdout

Stdout carries the build summary that tools parse programmatically (`Indexed N packages...`). Timing output goes to stderr so it never interferes with stdout consumers. Format:

```
Build timing:
  walk             12ms
  workspace-context 0ms
  diff              1ms
  parse            34ms
  remove-deleted    0ms
  recompute-internals 2ms
  update-hashes     0ms
  extract-symbols  89ms
  index-files      45ms
  total           183ms
```

Right-aligned durations would be nice but not required — left-aligned with a tab or padding is fine for a first pass.

### 3. Store total_duration_ms in shire_meta

After computing the total, store it as a new key in `shire_meta`:

```sql
INSERT OR REPLACE INTO shire_meta (key, value) VALUES ('total_duration_ms', ?1)
```

This lets users or tools query historical build performance. Only the total is stored — per-phase breakdown is ephemeral (stderr only). Storing per-phase would be easy to add later if needed, but is not worth the schema noise now.

### 4. Overall build timer wraps the entire function

A single `Instant::now()` at the top of `build_index` captures the overall wall-clock time. This is stored in `shire_meta` and printed as the "total" line. The sum of per-phase timings may be slightly less than the total due to interstitial work (variable setup, config overrides, git commit lookup) — this is expected and acceptable.

## Risks / Trade-offs

**Minimal risk.** `Instant::now()` is a monotonic clock read — nanosecond overhead, no syscall on most platforms. 18 calls (2 per phase) add negligible cost to a build that does filesystem walks and SQLite writes.

**Stderr noise.** The timing block adds ~12 lines to stderr on every build. This is acceptable for a developer tool but could be annoying for programmatic consumers that parse stderr. If this becomes an issue, a `--quiet` flag can suppress it later — but that is a non-goal for this change.

**Total vs sum-of-phases discrepancy.** The total timer includes interstitial code (variable binding, config overrides, git rev-parse). The sum of phases will be slightly less than the total. This is documented in the output and is not a bug.
