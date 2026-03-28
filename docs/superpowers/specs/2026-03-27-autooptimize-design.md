# Autooptimize: Autonomous Performance Optimization Loop

An autonomous optimization loop for shire, inspired by [Karpathy's autoresearch](https://github.com/karpathy/autoresearch). The agent proposes code changes to hot paths, benchmarks them against a real large repo, keeps statistically significant improvements, reverts failures, and loops indefinitely.

## Core Loop

```
LOOP FOREVER:
  1. Record current git state (SHA)
  2. Pick a target module and propose an optimization idea
  3. Implement the change
  4. cargo test (must pass — if not, one fix attempt, then revert)
  5. Run benchmark harness → get metrics JSON
  6. Compare to baseline (2-sigma threshold)
     - Improved → keep commit, update baseline
     - Regressed or neutral → git reset to previous SHA
  7. Log result to results.tsv
  8. Repeat
```

Two sequential phases:
1. **Build phase** — optimize `total_duration_ms` (full index of a large repo)
2. **Query phase** — optimize query latency (search_symbols, search_files, explore, etc.)

The agent runs phase 1 until three consecutive experiments show no improvement (suggesting diminishing returns), then switches to phase 2. The user can also force a phase switch at any time.

## Benchmark Harness

A standalone Rust binary (`[[bin]]` target in Cargo.toml, not a criterion bench) at `benches/autoresearch.rs` that programmatically exercises shire's internals. Invoked as `cargo run --release --bin autoresearch -- --phase build`.

### Build Benchmark

- Calls `config::load_config()` → `index::build_index()` against the test repo
- 5 iterations (first discarded as warmup)
- Deletes DB between runs to force full rebuilds
- Reports: median, p95, min, stddev of `total_duration_ms`
- Also captures: package count, symbol count, file count, DB file size

### Query Benchmark

- Builds the index once, then runs a fixed query set by calling the underlying `db::queries` functions directly (not through the MCP layer):
  - `search_symbols("parse")`, `search_symbols("Config")`
  - `search_files("mod.rs")`, `search_files("test")`
  - `explore("error handling")`
  - `package_dependencies` / `package_dependents`
- Each query runs 100 iterations
- Reports: median, p95, min per query type

### Output Format

JSON to stdout:

```json
{
  "phase": "build",
  "iterations": 4,
  "median_ms": 1423,
  "p95_ms": 1510,
  "min_ms": 1389,
  "stddev_ms": 42,
  "symbol_count": 48320,
  "file_count": 12841,
  "db_size_bytes": 15728640
}
```

### Statistical Significance

An improvement counts only if:

```
new_median < baseline_median - (2 * baseline_stddev)
```

This filters out noise and ensures only genuine improvements are kept.

### Test Repo

A single large polyglot repo, cloned once to `~/.cache/shire-bench/` and reused. Selection criteria:

- Must have 500+ source files across multiple languages shire supports
- Must have discoverable package manifests (package.json, go.mod, Cargo.toml, etc.)
- Must be publicly available and stable (no force-pushes that break reproducibility)
- Cloned at a pinned commit SHA to ensure reproducibility across runs

Candidates: VS Code (~30k files, TS/JSON), Kubernetes (~15k files, Go), or similar. The implementation step will evaluate candidates by running `shire build` against each and selecting the one that produces the most diverse workload (most packages, most languages, most symbols).

## Scope Constraints & Safety

### Single-Module Constraint

Each experiment targets exactly one module:

| Module | Scope |
|--------|-------|
| `src/index/` | Build orchestration, parallelism, incremental logic |
| `src/symbols/` | Tree-sitter extraction, query patterns, registry |
| `src/db/` | SQLite schema, pragmas, FTS config, queries |
| `src/mcp/` | Query-side only, relevant during phase 2 |

### Off-Limits (never modified)

- `benches/autoresearch.rs` — the measuring stick
- `tests/` — correctness tests are the safety net
- `src/config.rs` — config parsing isn't a hot path
- `Cargo.toml` — only modified when adding a new crate that yields >5% improvement

### Correctness Guard

`cargo test` must pass before benchmarking. If tests fail, the agent gets one fix attempt. If that also fails, revert and move on.

### Simplicity Criterion

"All else being equal, simpler is better." A 1% improvement that adds 50 lines of complexity isn't worth it. Removing code and getting equal results IS worth it. Prefer clean, idiomatic Rust.

### Dependencies Rule

New crates are allowed if the experiment shows >5% improvement on the target metric. The new dependency must be noted in the results.tsv `notes` column. No non-Rust dependencies (no shelling out to external tools).

### Crash Handling

If the benchmark crashes, read the error, attempt a fix, or revert and log "crash" in results.tsv.

## Results Tracking

### results.tsv

Located at repo root, gitignored. Columns:

| Column | Description |
|--------|-------------|
| `timestamp` | ISO 8601 |
| `experiment` | Short descriptive name |
| `module` | Target module (index, symbols, db, mcp) |
| `phase` | build or query |
| `median_ms` | Median duration from harness |
| `p95_ms` | 95th percentile duration |
| `baseline_ms` | Baseline median at time of experiment |
| `delta_pct` | Percentage change from baseline |
| `kept` | yes/no |
| `notes` | What was changed and why |

### Baseline Management

The first run establishes the baseline. After each kept improvement, the baseline updates. The agent always compares against the most recent kept state.

### Git Strategy

All work on a dedicated branch: `opt/autooptimize-YYYYMMDD`. Kept improvements form a clean chain of commits. The human reviews and cherry-picks/squashes when they return.

## Idea Generation Strategy

The skill provides a structured playbook ordered by typical impact.

### Build Phase (indexing)

1. **Parallelism tuning** — rayon thread pool size, chunk sizes, work distribution across packages vs files
2. **I/O reduction** — mmap for file reads during symbol extraction, batch file reads, reduce syscalls
3. **Hashing** — faster hash algorithms (blake3, xxhash vs SHA-256), hash fewer bytes for change detection
4. **SQLite write path** — batch insert sizes, transaction boundaries, prepared statement reuse, pragma tuning
5. **Tree-sitter** — parser reuse across files (one per thread vs per file), query cursor reuse, reduce allocations in extraction
6. **Memory/allocation** — reduce `Arc`/`String` cloning, arena allocation, pre-sized vectors
7. **FTS overhead** — defer FTS population strategy, trigger timing, incremental merge thresholds
8. **Algorithm** — smarter file walk ordering, skip-ahead heuristics, early termination

### Query Phase

1. **SQLite read path** — query plan optimization, covering indexes, pragma tuning for reads
2. **FTS tuning** — tokenizer config, prefix index lengths, rank function weights
3. **Connection pooling** — read-only connection reuse, reduce mutex contention in MCP server
4. **Result construction** — reduce allocations in result building, lazy field population
5. **Caching** — LRU cache for frequent queries, prepared statement warmup

### Anti-Patterns

- Don't add `unsafe` blocks for marginal gains
- Don't sacrifice readability for micro-optimizations
- Don't optimize cold paths (config parsing, CLI arg handling)

## Skill Protocol

The `/autooptimize` skill instructs the agent to:

1. **Setup** — Build the benchmark binary. Clone the test repo if not cached. Create the `opt/autooptimize-YYYYMMDD` branch. Establish baseline by running the harness 3 times.
2. **Idea generation** — Study the target module's code. Identify optimization opportunities using the playbook above. Review `results.tsv` to avoid repeating failed experiments.
3. **Implementation** — Make the change. Keep diffs small and focused. Commit with a descriptive message.
4. **Evaluation** — Run `cargo test`, then the benchmark harness. Parse JSON output. Compare to baseline using the 2-sigma threshold.
5. **Decision** — Keep or revert. Log to results.tsv either way.
6. **Never stop** — "The human might be asleep. You are autonomous. If you run out of ideas, think harder. Review what's been tried, read the code again, consider approaches from different angles."

## Deliverables

1. `benches/autoresearch.rs` — Rust benchmark binary
2. Synthetic/real test repo setup script or instructions
3. `/autooptimize` Claude Code skill (SKILL.md)
4. `.gitignore` entry for `results.tsv`
5. Documentation update in `docs/src/` if warranted
