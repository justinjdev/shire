## Context

`build_index` (line 398 of `src/index/mod.rs`) currently runs all 9 phases inline in a single function body. The function is ~350 lines long, mixes orchestration logic with phase-specific details, and passes state between phases through local variables. Several upcoming performance changes (timing, parallelization, transaction batching) need to wrap or modify individual phases, which is awkward when they're all inlined.

The file already has good factoring for helpers — `walk_manifests`, `diff_manifests`, `walk_files`, `associate_files_with_packages`, `upsert_package`, `upsert_symbols`, etc. are all separate functions. The gap is that the phases themselves (which compose these helpers with orchestration logic) are not extracted.

## Goals / Non-Goals

**Goals:**
- Extract each build phase into a named function with clear inputs and outputs
- Make `build_index` a readable ~50-line orchestrator
- Preserve identical behavior — this is a pure refactor
- Enable cleaner phase-level instrumentation, wrapping, and modification

**Non-Goals:**
- Changing any behavior or output
- Introducing new types or abstractions beyond what's needed
- Refactoring `db/queries.rs` (separate concern)
- Restructuring the module hierarchy (everything stays in `index/mod.rs`)

## Decisions

### Decision 1: Phase function signatures

Each phase function takes only what it needs — `conn`, `repo_root`, `config`, and phase-specific inputs. Return types capture what downstream phases need.

```
fn phase_walk(repo_root, config, parsers) -> Vec<WalkedManifest>
fn phase_workspace_context(walked) -> (HashMap<String,String>, HashSet<String>)
fn phase_diff(walked, conn) -> ManifestDiff
fn phase_parse(to_parse, conn, parsers, cargo_ws_deps, go_ws_dirs) -> (Vec<parsed>, Vec<failures>)
fn phase_remove_deleted(removed, conn) -> Result<()>
fn phase_recompute_internal(conn) -> Result<()>
fn phase_store_hashes(to_parse, conn) -> Result<()>
fn phase_extract_symbols(parsed, conn, repo_root) -> Result<()>
fn phase_source_incremental(unchanged, conn, repo_root) -> Result<usize>
fn phase_index_files(conn, repo_root, config) -> Result<usize>
```

Why: explicit inputs/outputs make dependencies between phases visible and enable future parallelization of independent phases.

### Decision 2: Keep everything in index/mod.rs

Don't split phases into separate files. They share types (`WalkedManifest`, `ManifestDiff`) and helpers that are private to the module. Splitting into files would require making internals `pub(crate)` for no benefit.

Why: the file goes from ~1230 lines to ~1230 lines (same code, just reorganized). Individual functions will be 20-60 lines each, which is readable.

### Decision 3: Introduce a BuildContext struct

Group the commonly-passed parameters into a context struct to reduce parameter passing:

```rust
struct BuildContext<'a> {
    conn: &'a Connection,
    repo_root: &'a Path,
    config: &'a Config,
}
```

Why: most phase functions need all three. A context struct keeps signatures clean without hiding what's available.

### Decision 4: Return a BuildSummary struct

Instead of accumulating counters as local variables, have `build_index` return a `BuildSummary`:

```rust
struct BuildSummary {
    num_added: usize,
    num_changed: usize,
    num_removed: usize,
    num_skipped: usize,
    num_source_reextracted: usize,
    num_symbols: usize,
    num_files: usize,
    failures: Vec<(String, String)>,
}
```

Why: separates summary computation from printing. Makes it easier to test the build pipeline without checking stdout. The printing logic at the end of `build_index` consumes this struct.

## Risks / Trade-offs

- **Risk:** Merge conflicts with in-flight performance changes. Mitigation: land this refactor first, rebase the others on top.
- **Trade-off:** Adds a `BuildContext` type that didn't exist before. Acceptable — it's a simple struct that reduces parameter noise.
- **Trade-off:** `BuildSummary` is a new type. Acceptable — it replaces scattered local variables with a testable struct.
