# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```sh
cargo build                     # Debug build
cargo build --release           # Release build
cargo test                      # All tests (unit + integration)
cargo test --lib                # Unit tests only
cargo test --test integration   # Integration tests only
cargo test config::tests        # Tests for a specific module
cargo check                     # Type check without building
```

The integration test (`tests/integration.rs`) builds the binary and runs it against fixture monorepos it creates in a temp directory.

## Architecture

Rust CLI (edition 2024) with four subcommands: `build`, `serve`, `watch`, `rebuild`.

**Data flow:** `config::load_config()` → `index::build_index()` → SQLite DB → `mcp::run_server()` (read-only)

### Key modules

- **index/** — Build orchestrator. Walks the repo, discovers manifests, parses them via the `ManifestParser` trait (one impl per ecosystem: npm, go, cargo, python, maven, gradle, perl, ruby), extracts symbols, writes to SQLite. Builds are incremental via SHA-256 content hashing of manifests and source files, with mtime pre-checks to skip unchanged packages entirely.
- **symbols/** — Source code symbol extraction. Uses tree-sitter for most languages, regex for Perl. Parallelized across packages with rayon. All extractors produce the same `SymbolInfo` struct.
- **db/** — SQLite with WAL mode, FTS5 full-text search (packages, symbols, files), triggers to maintain FTS indexes, batched multi-row INSERTs in transactions.
- **mcp/** — Read-only MCP server over stdio using the `rmcp` crate. 13 tools + 6 prompt templates for semantic codebase exploration.
- **watch/** — Unix-only background daemon. Uses Unix domain sockets (`.shire/watch.sock`) for IPC, PID file for process management, configurable debounce. Filters rebuilds by file relevance.

### Adding a new manifest parser

1. Create `src/index/<ecosystem>.rs` implementing the `ManifestParser` trait
2. Register it in `src/index/mod.rs` parser dispatch
3. Add the manifest filename to default config in `src/config.rs`
4. Add unit tests in the parser file, update integration test fixtures

### Adding a new symbol extractor

1. Create `src/symbols/<language>.rs` implementing extraction that returns `Vec<SymbolInfo>`
2. Register the language's file extensions in `src/symbols/walker.rs`
3. Wire it into the extraction dispatcher in `src/symbols/mod.rs`

## Platform Notes

- The `watch` module is Unix-only (Unix domain sockets, Unix signals, `kill` for process management). No Windows build target.
- Release builds target: `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`

## Releasing

1. Bump version in `Cargo.toml`
2. Commit and push to `main`
3. Tag with `git tag v<version>` and push the tag — this triggers `.github/workflows/release.yml`
4. The workflow builds for Linux x86_64, macOS aarch64, and macOS x86_64, then creates a GitHub Release
5. Update the Homebrew formula in `justinjdev/homebrew-shire` — update `version` and `sha256` hashes for each platform tarball

Tags containing `beta`, `alpha`, or `rc` are automatically marked as prereleases.

## Configuration

`shire.toml` at repo root. Key settings: `db_path`, `discovery.manifests`, `discovery.exclude`, `discovery.custom` rules, `symbols.exclude_extensions`, `watch.debounce_ms`, `[[packages]]` overrides.

## License

Apache-2.0
