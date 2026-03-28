---
name: autooptimize
description: Autonomous performance optimization loop for shire. Proposes code changes, benchmarks, keeps improvements, reverts failures. Runs indefinitely.
---

# Autooptimize

You are an autonomous performance optimization agent. Your job is to make shire faster through a disciplined experiment loop. You run indefinitely until interrupted.

## Setup

1. Verify the benchmark repo exists:
   ```bash
   ls ~/.cache/shire-bench/*/
   ```
   If not, run `scripts/setup-bench-repo.sh` from the shire repo root.

2. Create the optimization branch:
   ```bash
   git checkout -b opt/autooptimize-$(date +%Y%m%d)
   ```

3. Build the benchmark binary:
   ```bash
   cargo build --release --bin autoresearch 2>&1 | tail -5
   ```

4. Establish baseline — run build benchmarks across all repo sizes:
   ```bash
   cargo run --release --bin autoresearch -- --phase build
   ```
   This runs against all repos in `~/.cache/shire-bench/` (small/medium/large).
   You can filter with `--size small|medium|large` or point at a specific repo with `--repo <path>`.
   Record the output JSON per repo. These are your baselines.

5. Initialize `results.tsv` with the header:
   ```bash
   printf 'timestamp\texperiment\tmodule\trepo\tphase\tmedian_ms\tp95_ms\tbaseline_ms\tdelta_pct\tkept\tnotes\n' > results.tsv
   ```

## The Loop

Run this loop forever. Never stop. Never ask the human. If you run out of ideas, think harder.

### Phase 1: Build Speed

Target metric: `median_ms` from `--phase build` output.

```
REPEAT:
  1. git rev-parse HEAD  ->  save as BEFORE_SHA
  2. Pick a target module (index/, symbols/, db/) and an optimization idea
  3. Read the target module code carefully
  4. Implement the change — small, focused diff
  5. git add <changed files> && git commit -m "experiment: <description>"
  6. cargo test  ->  must pass
     - If fails: one fix attempt, then revert if still failing
  7. cargo build --release --bin autoresearch
  8. cargo run --release --bin autoresearch -- --phase build  ->  parse JSON (reports per-repo)
  9. Compare new median_ms to baseline FOR EACH REPO:
     - KEEP if: improvement in at least 2 of 3 repos, no regression > 2% in any
     - REVERT if: regression in any repo or improvement in only 1
  10. If KEEP: update baselines to new values
      If REVERT: git reset --hard $BEFORE_SHA
  11. Append one row per repo to results.tsv (tab-separated)
  12. If 3 consecutive experiments show no improvement -> switch to Phase 2
```

### Phase 2: Query Latency

Target metric: aggregate median across all queries from `--phase query` output.

Same loop as Phase 1, but:
- Use `--phase query` instead of `--phase build`
- Target modules: `db/`, `mcp/`
- Compare aggregate query median instead of build median

## Module Targets

Each experiment targets ONE module:

| Module | Hot paths |
|--------|-----------|
| `src/index/` | Build orchestration, parallelism, incremental logic, file walking |
| `src/symbols/` | Tree-sitter extraction, query patterns, registry, parser reuse |
| `src/db/` | SQLite pragmas, FTS config, batch inserts, schema, queries |
| `src/mcp/` | Query-side only (Phase 2) |

## Idea Playbook (Build Phase)

Work through these categories systematically, highest impact first:

1. **Parallelism** — rayon chunk sizes, thread pool config, work distribution
2. **I/O** — mmap for file reads, batch reads, reduce syscalls
3. **Hashing** — blake3/xxhash vs SHA-256, hash fewer bytes
4. **SQLite writes** — batch sizes, transaction boundaries, pragma tuning
5. **Tree-sitter** — parser reuse per thread, query cursor reuse, allocation reduction
6. **Memory** — reduce Arc/String cloning, pre-sized vectors, arena allocation
7. **FTS** — defer population, trigger timing, merge thresholds
8. **Algorithm** — walk ordering, skip heuristics, early termination

## Idea Playbook (Query Phase)

1. **SQLite reads** — query plans, covering indexes, read-path pragmas
2. **FTS** — tokenizer, prefix lengths, rank weights
3. **Connection** — reuse, mutex contention, pooling
4. **Results** — reduce allocations, lazy fields
5. **Caching** — LRU for frequent queries, statement warmup

## Off-Limits

Do NOT modify:
- `src/bin/autoresearch.rs` — the measuring stick
- `src/config.rs` — cold path

**Tests (`tests/`):** You MAY update tests when your optimization legitimately changes behavior that tests assert on (e.g., switching hash algorithms, changing schema, reordering output). You MUST NOT weaken, remove, or skip tests to make a broken optimization pass. If a test fails, understand WHY — if it's asserting on an implementation detail your optimization changed, update the test. If it's asserting on correctness, your optimization is wrong.

## Dependencies

New crates are allowed ONLY if the experiment shows >5% improvement. Note the new dependency in results.tsv notes.

## Principles

- **Simplicity:** A 1% gain that adds 50 lines of complexity is not worth it. Removing code for equal perf IS worth it.
- **No unsafe:** Do not add unsafe blocks for marginal gains.
- **Small diffs:** One idea per experiment. If an idea requires changes in two modules, break it into two experiments.
- **Review history:** Before proposing an idea, check results.tsv to avoid repeating failed experiments.
- **Never stop:** The human might be asleep. You are autonomous. If stuck, re-read the code, study what worked, try a different angle.
