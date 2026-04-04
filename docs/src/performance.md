# Performance

Shire is designed to index large monorepos quickly and answer queries instantly. This page documents benchmark methodology, results, and how to reproduce them.

## Test repos

Benchmarks run against three real-world open-source monorepos covering a range of sizes and ecosystems:

| Repo | Size | Packages | Symbols | Files | Primary languages |
|---|---|---|---|---|---|
| [turborepo](https://github.com/vercel/turborepo) | small | 400 | 10,686 | 5,451 | Rust, TypeScript, Go |
| [grafana](https://github.com/grafana/grafana) | medium | 28 | 35,104 | 14,054 | Go, TypeScript |
| [kubernetes](https://github.com/kubernetes/kubernetes) | large | 34 | 78,458 | 18,275 | Go |

## Build performance

Full rebuild (no incremental cache), median of 4 iterations after a warmup run:

| Repo | Median | Min | P95 | Std dev |
|---|---|---|---|---|
| turborepo | 571ms | 525ms | 678ms | 58ms |
| grafana | 1,150ms | 1,025ms | 1,172ms | 58ms |
| kubernetes | 1,703ms | 1,607ms | 1,897ms | 108ms |

Build time scales roughly linearly with symbol count. The pipeline is parallelized with rayon across packages and files, with batched multi-row SQLite inserts within explicit transactions.

Incremental builds are significantly faster -- only packages with changed source files are re-extracted, and an mtime pre-check skips SHA-256 computation entirely for untouched packages.

## Query performance

Median latency over 100 iterations per query:

| Query | Small | Medium | Large |
|---|---|---|---|
| `search_symbols("parse")` | 0.09ms | 0.09ms | 0.04ms |
| `search_symbols("Config")` | 0.20ms | 0.29ms | 1.01ms |
| `search_files("mod")` | 0.05ms | 0.03ms | 0.04ms |
| `search_files("test")` | 0.07ms | 0.60ms | 1.99ms |
| `list_packages(None)` | 0.11ms | 0.01ms | 0.01ms |

All queries use SQLite FTS5 full-text search with `unicode61` tokenizer and prefix indexes. Query latency depends primarily on result set size, not total index size.

## Reproducing benchmarks

Shire includes an `autoresearch` binary for reproducible benchmarking.

### Setup

Run the benchmark repo setup script to clone and prepare the test repos:

```sh
scripts/setup-bench-repo.sh
```

This clones the three repos into `~/.cache/shire-bench/` and creates a `shire.toml` in each.

### Running benchmarks

```sh
# Build the benchmark binary
cargo build --release --bin autoresearch

# Run build benchmarks (all repos)
cargo run --release --bin autoresearch -- --phase build

# Run query benchmarks (all repos)
cargo run --release --bin autoresearch -- --phase query

# Filter by repo size
cargo run --release --bin autoresearch -- --phase build --size small

# Point at a specific repo
cargo run --release --bin autoresearch -- --phase build --repo /path/to/repo
```

Build benchmarks run 5 iterations (1 warmup + 4 measured) per repo. Query benchmarks run 100 iterations per query. Results are printed as JSON to stdout.

### Environment notes

- Results vary by machine (CPU, disk speed, available memory)
- Close other applications for more stable measurements
- The warmup iteration primes filesystem caches and SQLite page cache
- Numbers in this document were captured on an Apple M-series Mac
