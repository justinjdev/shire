# Quick Start

## Index a monorepo

```sh
shire build --root /path/to/repo
```

## Rebuild from scratch

Ignore cached hashes and re-parse everything:

```sh
shire build --root /path/to/repo --force
```

## Write the index to a custom location

```sh
shire build --root /path/to/repo --db /tmp/my-index.db
```

## Start the MCP server

```sh
shire serve
```

The index is written to `.shire/index.db` inside the repo root by default. You can override this with `--db` on the build command or `db_path` in `shire.toml` (see [Configuration](./configuration.md)).

## Incremental builds

Subsequent builds are **incremental** — only manifests whose content has changed (by SHA-256 hash) are re-parsed. Source files are also tracked: if source files change without a manifest change, symbols are re-extracted automatically. An **mtime pre-check** skips SHA-256 computation entirely for packages whose source files haven't been touched since the last build.

File indexing is also incremental — a file-tree hash detects structural changes, skipping Phase 9 entirely when no files have been added, removed, or resized.

## Performance

Symbol extraction and source hashing are **parallelized** across packages using rayon for multi-core throughput. All database writes use **batched multi-row INSERTs** within explicit transactions for maximum SQLite throughput. A per-phase **timing breakdown** is printed to stderr after each build. The server reads from the database in read-only mode.
