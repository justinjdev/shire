# Autooptimize Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an autonomous performance optimization loop for shire that proposes code changes, benchmarks them against a real large repo, keeps statistically significant improvements, and reverts failures.

**Architecture:** A Rust benchmark binary (`src/bin/autoresearch.rs`) calls shire's indexing and query functions directly. A Claude Code skill (`/autooptimize`) drives the autonomous loop: propose change, test, benchmark, keep/revert, log, repeat. The crate must be restructured as a library + binary to allow the benchmark to import shire's modules.

**Tech Stack:** Rust (edition 2024), SQLite, tree-sitter, serde_json for benchmark output, clap for benchmark CLI args

**Spec:** `docs/superpowers/specs/2026-03-27-autooptimize-design.md`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|---------------|
| Create | `src/lib.rs` | Re-export modules for library consumers (benchmark binary) |
| Modify | `src/main.rs` | Import from library crate instead of declaring modules |
| Modify | `Cargo.toml` | Add `[[bin]]` target for benchmark, add `[lib]` section |
| Create | `src/bin/autoresearch.rs` | Benchmark harness binary (build + query phases) |
| Create | `scripts/setup-bench-repo.sh` | Clone and pin the benchmark test repo |
| Create | `~/.claude/skills/autooptimize/SKILL.md` | Claude Code skill for the autonomous loop |
| Modify | `.gitignore` | Add `results.tsv` |

---

### Task 1: Restructure as Library + Binary

Shire is currently a binary-only crate (`mod` declarations in `main.rs`). The benchmark binary needs to import shire's modules, so we must add a `lib.rs` that re-exports them.

**Files:**
- Create: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Create `src/lib.rs`**

This file re-exports all modules that the benchmark binary needs:

```rust
pub mod config;
pub mod db;
pub mod git;
pub mod index;
pub mod symbols;
pub mod logging;

// These are only needed by the CLI binary, not the benchmark
mod init;
mod install;
mod mcp;
mod rag;
mod watch;
```

Note: `init`, `install`, `mcp`, `rag`, and `watch` stay private — the benchmark doesn't need them. However, if any of these modules are referenced by the public modules (e.g., `index` references `rag`), they may need to be public too. Check compilation in Step 4.

- [ ] **Step 2: Update `src/main.rs`**

Remove all `mod` declarations and replace with imports from the library crate:

```rust
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

// These are now provided by the library crate (src/lib.rs)
use shire::config;
use shire::db;
use shire::git;
use shire::index;
use shire::logging;

// These remain as local modules since they're CLI-only
mod init;
mod install;
mod mcp;
mod rag;
mod symbols;
mod watch;
```

Important: The exact set of `use shire::X` vs `mod X` depends on which modules `main.rs` references directly. If `main.rs` calls functions from `mcp`, `rag`, `watch`, etc., those must either stay as local `mod` declarations or be re-exported from `lib.rs`. The key is that `config`, `db`, `git`, `index`, and `symbols` must come from `lib.rs` so the benchmark can use them.

- [ ] **Step 3: Add `[lib]` section to `Cargo.toml`**

Add these sections to `Cargo.toml`:

```toml
[lib]
name = "shire"
path = "src/lib.rs"

[[bin]]
name = "shire"
path = "src/main.rs"
```

- [ ] **Step 4: Verify compilation**

Run:
```bash
output=$(cargo check 2>&1) || { echo "$output" | tail -40; false; }
echo "check passed"
```

Expected: Compiles successfully. If there are import errors (e.g., `index` module references `rag` internally), resolve by making additional modules public in `lib.rs` or using `cfg` attributes. Iterate until `cargo check` passes.

- [ ] **Step 5: Run tests**

Run:
```bash
output=$(cargo test 2>&1) || { echo "$output" | tail -40; false; }
echo "tests passed"
```

Expected: All existing tests pass. This restructuring should be purely organizational — no behavior changes.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/main.rs Cargo.toml
git commit -m "refactor: restructure as library + binary crate for benchmark support"
```

---

### Task 2: Benchmark Test Repo Setup

Create a script that clones a large polyglot repo to `~/.cache/shire-bench/` at a pinned commit for reproducible benchmarks.

**Files:**
- Create: `scripts/setup-bench-repo.sh`

- [ ] **Step 1: Evaluate candidate repos**

Run `shire build` against a few candidate repos to find the best benchmark corpus. The ideal repo has: many packages, multiple languages, many source files with symbols.

Candidates to evaluate:
- VS Code (`microsoft/vscode`) — ~30k files, TypeScript-heavy, has package.json manifests
- Kubernetes (`kubernetes/kubernetes`) — ~15k files, Go-heavy, has go.mod manifests
- GitLab (`gitlabhq/gitlabhq`) — Ruby + JS, has Gemfile and package.json

Clone each to a temp directory, run `shire build --root <path>`, and compare:
- Number of packages discovered
- Number of symbols extracted
- Number of files indexed
- Build duration
- Language diversity

Pick the repo with the most diverse workload.

- [ ] **Step 2: Write `scripts/setup-bench-repo.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail

# Configuration — update REPO_URL, COMMIT_SHA, and REPO_NAME after evaluation in Step 1
REPO_URL="https://github.com/microsoft/vscode.git"  # REPLACE with winner from Step 1
COMMIT_SHA="abc123def456"                             # REPLACE with pinned commit from Step 1
REPO_NAME="vscode"                                    # REPLACE with repo name from Step 1

BENCH_DIR="${HOME}/.cache/shire-bench"
REPO_DIR="${BENCH_DIR}/${REPO_NAME}"

if [ -d "${REPO_DIR}/.git" ]; then
    echo "Benchmark repo already exists at ${REPO_DIR}"
    cd "${REPO_DIR}"
    CURRENT_SHA=$(git rev-parse HEAD)
    if [ "${CURRENT_SHA}" = "${COMMIT_SHA}" ]; then
        echo "Already at pinned commit ${COMMIT_SHA}"
        exit 0
    else
        echo "Resetting to pinned commit ${COMMIT_SHA}"
        git fetch origin
        git checkout "${COMMIT_SHA}"
        exit 0
    fi
fi

echo "Cloning ${REPO_URL} to ${REPO_DIR}..."
mkdir -p "${BENCH_DIR}"
git clone --no-checkout "${REPO_URL}" "${REPO_DIR}"
cd "${REPO_DIR}"
git checkout "${COMMIT_SHA}"
echo "Benchmark repo ready at ${REPO_DIR} (commit ${COMMIT_SHA})"
```

- [ ] **Step 3: Make executable and test**

```bash
chmod +x scripts/setup-bench-repo.sh
```

Run the script and verify the repo is cloned correctly. This will take a few minutes for a large repo.

- [ ] **Step 4: Commit**

```bash
git add scripts/setup-bench-repo.sh
git commit -m "feat: add benchmark test repo setup script"
```

---

### Task 3: Benchmark Harness — Build Phase

The core benchmark binary that measures indexing performance.

**Files:**
- Create: `src/bin/autoresearch.rs`
- Modify: `Cargo.toml` (add `[[bin]]` target)

- [ ] **Step 1: Add benchmark binary target to `Cargo.toml`**

The `[[bin]]` for `shire` was added in Task 1. Now add the autoresearch target:

```toml
[[bin]]
name = "autoresearch"
path = "src/bin/autoresearch.rs"
```

- [ ] **Step 2: Write the benchmark harness — argument parsing and main structure**

Create `src/bin/autoresearch.rs`:

```rust
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Serialize)]
struct BuildResult {
    phase: String,
    iterations: usize,
    median_ms: f64,
    p95_ms: f64,
    min_ms: f64,
    stddev_ms: f64,
    package_count: i64,
    symbol_count: i64,
    file_count: i64,
    db_size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct QueryResult {
    phase: String,
    queries: Vec<QueryBenchmark>,
}

#[derive(Debug, Serialize)]
struct QueryBenchmark {
    name: String,
    iterations: usize,
    median_ms: f64,
    p95_ms: f64,
    min_ms: f64,
}

fn parse_args() -> (String, PathBuf) {
    let args: Vec<String> = std::env::args().collect();
    let mut phase = String::from("build");
    let mut repo_dir = dirs_or_default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--phase" => {
                i += 1;
                phase = args[i].clone();
            }
            "--repo" => {
                i += 1;
                repo_dir = PathBuf::from(&args[i]);
            }
            _ => {}
        }
        i += 1;
    }
    (phase, repo_dir)
}

fn dirs_or_default() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    // Find the first directory in ~/.cache/shire-bench/
    let bench_dir = PathBuf::from(&home).join(".cache/shire-bench");
    if bench_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&bench_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() && entry.path().join(".git").exists() {
                    return entry.path();
                }
            }
        }
    }
    bench_dir
}

fn main() -> Result<()> {
    let (phase, repo_dir) = parse_args();

    if !repo_dir.exists() {
        anyhow::bail!(
            "Benchmark repo not found at {}. Run scripts/setup-bench-repo.sh first.",
            repo_dir.display()
        );
    }

    match phase.as_str() {
        "build" => {
            let result = bench_build(&repo_dir)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        "query" => {
            let result = bench_queries(&repo_dir)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        other => anyhow::bail!("Unknown phase: {other}. Use 'build' or 'query'."),
    }

    Ok(())
}

fn bench_build(repo_dir: &Path) -> Result<BuildResult> {
    let config = shire::config::load_config(repo_dir)
        .unwrap_or_else(|_| shire::config::Config::default());
    let db_path = repo_dir.join(".shire/bench.db");
    let iterations = 5;
    let mut durations_ms: Vec<f64> = Vec::with_capacity(iterations);

    for i in 0..iterations {
        // Delete DB to force full rebuild
        let _ = std::fs::remove_file(&db_path);

        let start = Instant::now();
        shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path))?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        if i == 0 {
            eprintln!("warmup: {elapsed:.1}ms (discarded)");
            continue;
        }

        eprintln!("run {i}: {elapsed:.1}ms");
        durations_ms.push(elapsed);
    }

    // Collect counts from the last run's DB
    let conn = shire::db::open_readonly(&db_path)?;
    let package_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM packages", [], |r| r.get(0))
        .unwrap_or(0);
    let symbol_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
        .unwrap_or(0);
    let file_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap_or(0);
    let db_size_bytes = std::fs::metadata(&db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    durations_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = durations_ms.len();
    let median = durations_ms[n / 2];
    let p95 = durations_ms[(n as f64 * 0.95) as usize].min(*durations_ms.last().unwrap());
    let min = durations_ms[0];
    let mean = durations_ms.iter().sum::<f64>() / n as f64;
    let variance = durations_ms.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n as f64;
    let stddev = variance.sqrt();

    // Clean up
    let _ = std::fs::remove_file(&db_path);

    Ok(BuildResult {
        phase: "build".to_string(),
        iterations: n,
        median_ms: (median * 10.0).round() / 10.0,
        p95_ms: (p95 * 10.0).round() / 10.0,
        min_ms: (min * 10.0).round() / 10.0,
        stddev_ms: (stddev * 10.0).round() / 10.0,
        package_count,
        symbol_count,
        file_count,
        db_size_bytes,
    })
}

fn bench_queries(repo_dir: &Path) -> Result<QueryResult> {
    let config = shire::config::load_config(repo_dir)
        .unwrap_or_else(|_| shire::config::Config::default());
    let db_path = repo_dir.join(".shire/bench.db");

    // Build index first if DB doesn't exist
    if !db_path.exists() {
        eprintln!("Building index for query benchmark...");
        shire::index::build_index_quiet(repo_dir, &config, true, Some(&db_path))?;
    }

    let conn = shire::db::open_readonly(&db_path)?;
    let iterations = 100;

    let queries: Vec<(&str, Box<dyn Fn(&rusqlite::Connection) -> Result<()>>)> = vec![
        ("search_symbols(\"parse\")", Box::new(|conn| {
            shire::db::queries::search_symbols(conn, "parse", None, None, 50)?;
            Ok(())
        })),
        ("search_symbols(\"Config\")", Box::new(|conn| {
            shire::db::queries::search_symbols(conn, "Config", None, None, 50)?;
            Ok(())
        })),
        ("search_files(\"mod\")", Box::new(|conn| {
            shire::db::queries::search_files(conn, "mod", None, None)?;
            Ok(())
        })),
        ("search_files(\"test\")", Box::new(|conn| {
            shire::db::queries::search_files(conn, "test", None, None)?;
            Ok(())
        })),
        ("list_packages", Box::new(|conn| {
            shire::db::queries::list_packages(conn, None)?;
            Ok(())
        })),
    ];

    let mut benchmarks = Vec::new();

    for (name, query_fn) in &queries {
        let mut durations_ms: Vec<f64> = Vec::with_capacity(iterations);

        for _ in 0..iterations {
            let start = Instant::now();
            query_fn(&conn)?;
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            durations_ms.push(elapsed);
        }

        durations_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = durations_ms.len();
        let median = durations_ms[n / 2];
        let p95 = durations_ms[(n as f64 * 0.95) as usize];
        let min = durations_ms[0];

        eprintln!("{name}: median={median:.3}ms p95={p95:.3}ms min={min:.3}ms");

        benchmarks.push(QueryBenchmark {
            name: name.to_string(),
            iterations: n,
            median_ms: (median * 1000.0).round() / 1000.0,
            p95_ms: (p95 * 1000.0).round() / 1000.0,
            min_ms: (min * 1000.0).round() / 1000.0,
        });
    }

    Ok(QueryResult {
        phase: "query".to_string(),
        queries: benchmarks,
    })
}
```

- [ ] **Step 3: Verify compilation**

Run:
```bash
output=$(cargo check --bin autoresearch 2>&1) || { echo "$output" | tail -40; false; }
echo "check passed"
```

Fix any compilation errors. Common issues:
- Module visibility: if `build_index_quiet` isn't pub in lib.rs, make it pub
- Missing re-exports: if `db::queries` isn't accessible, ensure `db` module has `pub mod queries`
- rusqlite type imports: the benchmark needs `rusqlite::Connection` — may need rusqlite as a direct dependency or re-export

- [ ] **Step 4: Run against the benchmark repo**

```bash
cargo run --release --bin autoresearch -- --phase build
```

This will take several minutes. Verify JSON output is valid and contains reasonable values.

- [ ] **Step 5: Commit**

```bash
git add src/bin/autoresearch.rs Cargo.toml
git commit -m "feat: add autoresearch benchmark harness (build + query phases)"
```

---

### Task 4: Add `results.tsv` to `.gitignore`

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Add entry**

Append `results.tsv` to `.gitignore`:

```
results.tsv
```

- [ ] **Step 2: Commit**

```bash
git add .gitignore
git commit -m "chore: gitignore results.tsv for autooptimize loop"
```

---

### Task 5: Write the `/autooptimize` Skill

The Claude Code skill that drives the autonomous optimization loop.

**Files:**
- Create: `~/.claude/skills/autooptimize/SKILL.md`

- [ ] **Step 1: Write `SKILL.md`**

```markdown
---
name: autooptimize
description: Autonomous performance optimization loop for shire. Proposes code changes, benchmarks, keeps improvements, reverts failures. Runs indefinitely.
---

# Autooptimize

You are an autonomous performance optimization agent. Your job is to make shire faster through a disciplined experiment loop. You run indefinitely until interrupted.

## Setup

1. Verify the benchmark repo exists:
   ```bash
   ls ~/.cache/shire-bench/*/  # should contain a git repo
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

4. Establish baseline — run the build benchmark 3 times, take the median as baseline:
   ```bash
   cargo run --release --bin autoresearch -- --phase build
   ```
   Record the output JSON. This is your baseline. Save it mentally — you'll compare every experiment against it.

5. Initialize `results.tsv` with the header:
   ```
   timestamp\texperiment\tmodule\tphase\tmedian_ms\tp95_ms\tbaseline_ms\tdelta_pct\tkept\tnotes
   ```

## The Loop

Run this loop forever. Never stop. Never ask the human. If you run out of ideas, think harder.

### Phase 1: Build Speed

Target metric: `median_ms` from `--phase build` output.

```
REPEAT:
  1. git rev-parse HEAD  →  save as BEFORE_SHA
  2. Pick a target module (index/, symbols/, db/) and an optimization idea
  3. Read the target module code carefully
  4. Implement the change — small, focused diff
  5. git add <changed files> && git commit -m "experiment: <description>"
  6. cargo test  →  must pass
     - If fails: one fix attempt, then revert if still failing
  7. cargo build --release --bin autoresearch
  8. cargo run --release --bin autoresearch -- --phase build  →  parse JSON
  9. Compare new median_ms to baseline:
     - KEEP if: new_median < baseline_median - (2 * baseline_stddev)
     - REVERT if: new_median >= baseline_median - (2 * baseline_stddev)
  10. If KEEP: update baseline to new values
      If REVERT: git reset --hard $BEFORE_SHA
  11. Append result to results.tsv
  12. If 3 consecutive experiments show no improvement → switch to Phase 2
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
- `tests/` — the safety net
- `src/config.rs` — cold path

## Dependencies

New crates are allowed ONLY if the experiment shows >5% improvement. Note the new dependency in results.tsv notes.

## Principles

- **Simplicity:** A 1% gain that adds 50 lines of complexity is not worth it. Removing code for equal perf IS worth it.
- **No unsafe:** Don't add unsafe blocks for marginal gains.
- **Small diffs:** One idea per experiment. If an idea requires changes in two modules, break it into two experiments.
- **Review history:** Before proposing an idea, check results.tsv to avoid repeating failed experiments.
- **Never stop:** The human might be asleep. You are autonomous. If stuck, re-read the code, study what worked, try a different angle.
```

- [ ] **Step 2: Verify skill is discoverable**

Run Claude Code and check that `/autooptimize` appears in the skill list. If skills need registration in a config file, add it.

- [ ] **Step 3: Commit the skill**

```bash
cd ~/.claude/skills
git add autooptimize/SKILL.md
git commit -m "feat: add /autooptimize skill for autonomous perf optimization"
```

Note: If `~/.claude/skills` is not a git repo, skip the commit. The skill file just needs to exist at the path.

---

### Task 6: End-to-End Smoke Test

Verify the full pipeline works before handing off to autonomous use.

**Files:** None (testing only)

- [ ] **Step 1: Run setup script**

```bash
cd /Users/justin/git/shire
bash scripts/setup-bench-repo.sh
```

Verify the repo is cloned to `~/.cache/shire-bench/<repo-name>/`.

- [ ] **Step 2: Run build benchmark**

```bash
cargo run --release --bin autoresearch -- --phase build
```

Verify valid JSON output with reasonable values.

- [ ] **Step 3: Run query benchmark**

```bash
cargo run --release --bin autoresearch -- --phase query
```

Verify valid JSON output with per-query timings.

- [ ] **Step 4: Simulate one experiment cycle**

Manually test the keep/revert flow:

```bash
# Record baseline
BEFORE=$(git rev-parse HEAD)

# Make a trivial change (e.g., change a comment in src/index/mod.rs)
# Commit it
git add src/index/mod.rs
git commit -m "experiment: test-cycle"

# Run tests
cargo test

# Run benchmark
cargo run --release --bin autoresearch -- --phase build

# Revert (since this was a no-op change)
git reset --hard $BEFORE
```

Verify the cycle completes without errors.

- [ ] **Step 5: Test the skill invocation**

Open a new Claude Code session and run `/autooptimize`. Verify it:
1. Checks for the benchmark repo
2. Creates the optimization branch
3. Establishes a baseline
4. Begins the experiment loop

Let it run for 2-3 experiments to verify the full loop works, then interrupt.
