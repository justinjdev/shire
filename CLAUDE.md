# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```sh
cargo build                     # Debug build
cargo build --release           # Release build
cargo test                      # Unit + integration tests (default features)
cargo test --all-features       # ...plus the bench-gated autoresearch harness (what CI runs)
cargo test --lib                # Unit tests only
cargo test --test integration   # Integration tests only
cargo test config::tests        # Tests for a specific module
cargo check                     # Type check without building
```

The crate has no default features. The optional, non-default `bench` feature gates
the internal benchmark harness (`src/bin/autoresearch.rs`), so it is not built or
installed unless explicitly requested:

```sh
cargo build --features bench --bin autoresearch
cargo run --features bench --bin autoresearch -- --phase quality
```

The integration test (`tests/integration.rs`) runs the `shire` binary cargo built for
the test run (via `CARGO_BIN_EXE_shire`) against fixture monorepos it creates in a temp
directory.

Changes should include unit tests covering new or modified logic. Run `cargo test --lib` to
verify before committing — and `cargo test --all-features` when you touch
`src/bin/autoresearch.rs`, whose tests live in a `bench`-gated bin target that `--lib` does
not build.

## Architecture

Rust CLI (edition 2024) with subcommands: `build`, `serve`, `watch`, `rebuild`, `init`, `install`, `uninstall`, `clean`.

**Data flow:** `config::load_config()` → `index::build_index()` → SQLite DB → `mcp::run_server()` (read-only, or on-demand rebuild with `--root`)

### Key modules

- **index/** — Build orchestrator. Walks the repo, discovers manifests, parses them via the `ManifestParser` trait (one impl per ecosystem: npm, go, cargo, python, maven, gradle, perl, ruby), extracts symbols, writes to SQLite. Builds are incremental at file granularity via SHA-256 content hashing (`file_hashes` table), with mtime pre-checks to skip unchanged packages entirely. Single-pass read+hash+extract avoids double file reads. File tree walk capped at 500k files. Detects proto→generated-code boundary edges during file walking using stem matching and package-scope filtering.
- **symbols/** — Source code symbol extraction. Uses tree-sitter query patterns + language-specific hooks for most languages (TS/JS, Go, Rust, Python, Java, Kotlin, Dart, C, C++, C#, Swift, PHP, Scala, Zig, Protobuf, Bash, R, Haskell, YAML, SQL, HCL, TOML, Perl, Ruby, OCaml, Lua, Elixir, Clojure, Erlang, Julia, Gleam, Odin, Nix, Nim), regex for COBOL. Parallelized across packages and within packages with rayon. Tree-sitter queries compiled once per language via `OnceLock`. All extractors produce the same `SymbolInfo` struct (uses `Arc<str>` for file paths). Cross-reference extraction (call, type, import, impl) is supported for 8 tier-1 languages: Go, Python, Java, TypeScript, JavaScript, Perl, Ruby, Scala — captured via `@reference.*` tree-sitter captures and written to the `symbol_refs` table.
- **db/** — SQLite with WAL mode, FTS5 full-text search (packages, symbols, files, docs) with custom tokenizers (`unicode61` with `tokenchars` for underscores/hyphens) and prefix indexes. FTS triggers dropped/recreated during bulk operations for performance. Schema versioned via `shire_meta` with automatic FTS migration on upgrade. Read-only connections use `query_only` and `prepare_cached`. The `symbol_refs` table stores cross-reference records (call, type, import, impl) with B-tree indexes on `name`, `file_id`, and `enclosing_symbol`. Each ref stores a compact `file_id INTEGER REFERENCES files(id)` instead of a duplicated path string — `phase_index_files` runs before symbol extraction so the `files` table is populated when refs are inserted, and read queries JOIN `files` to resolve `file_path`. Records are rebuilt at file granularity alongside symbol extraction.
- **mcp/** — MCP server over stdio using the `rmcp` crate. 17 tools + 2 prompt templates for semantic codebase exploration. Supports on-demand reindexing via `serve --root` (checks `.git/index` mtime for staleness). Four reference tools: `symbol_references` (all refs to a name), `symbol_callers` (call-site callers), `symbol_callees` (outbound call graph), `change_impact` (refs + dep graph for blast-radius analysis). Two boundary tools: `schema_consumers` (proto → generated), `generated_from` (generated → proto). The `reference_audit` prompt guides refactor-safety analysis.
- **watch/** — Unix-only background daemon. Uses Unix domain sockets (`.shire/watch.sock`) for IPC, PID file for process management, configurable debounce. Filters rebuilds by file relevance.

### Adding a new manifest parser

1. Create `src/index/<ecosystem>.rs` implementing the `ManifestParser` trait
2. Register it in `src/index/mod.rs` parser dispatch
3. Add the manifest filename to default config in `src/config.rs`
4. Add unit tests in the parser file, update integration test fixtures

### Adding a new symbol extractor

**Tree-sitter languages (preferred):**

1. Create `src/symbols/queries/<language>.scm` with tree-sitter query patterns using `@name` and `@definition.<kind>` captures
2. Create `src/symbols/hooks/<language>.rs` implementing `LanguageHooks` (visibility, signatures, params, return types, post-processing)
3. Add `pub mod <language>;` to `src/symbols/hooks/mod.rs`
4. Add a `LanguageEntry` to `src/symbols/registry.rs` with the grammar, query, hooks, and file extensions
5. Add extensions to `all_extensions()` in `src/symbols/walker.rs` and add assertions in `test_all_extensions`
6. Add the language to the symbols module description in this file (the parenthetical language list)
7. Add a row to the Symbol extraction table in `docs/src/ecosystems.md`

**Regex-based languages (when tree-sitter grammar is impractical):**

1. Create `src/symbols/<language>.rs` with `pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
2. Add `pub mod <language>;` to `src/symbols/mod.rs`
3. Add a match arm in `extract_file()` in `src/symbols/registry.rs`
4. Add extensions to `all_extensions()` in `src/symbols/walker.rs` and add assertions in `test_all_extensions`
5. Add the language to the symbols module description in this file (the parenthetical language list)
6. Add a row to the Symbol extraction table in `docs/src/ecosystems.md`

### Adding reference extraction to a language

Only applicable to languages that already have tree-sitter-based symbol extraction.

1. Add `@reference.call`, `@reference.type`, `@reference.import`, `@reference.impl` captures to the language's `.scm` file alongside existing `@definition.X` captures
2. In the language's hooks file (`src/symbols/hooks/<lang>.rs`), set `reference_hooks: Some(ReferenceHooks { enclosing_ancestors: &[...], reference_stoplist: &[...] })` with the grammar's function/method/class node kinds and language built-ins to skip
3. Add unit tests in `src/symbols/tests.rs` asserting each ref kind is extracted
4. Add a row to the Reference extraction table in `docs/src/ecosystems.md`

## Platform Notes

- The `watch` module is Unix-only (Unix domain sockets, Unix signals, `kill` for process management). No Windows build target.
- Release builds target: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`

## Releasing

1. Bump version in `Cargo.toml`
2. Commit and push to `main`
3. Tag with `git tag v<version>` and push the tag — this triggers `.github/workflows/release.yml`
4. The workflow builds for Linux x86_64, macOS aarch64, and macOS x86_64, then creates a GitHub Release
5. Update the Homebrew formula in `justinjdev/homebrew-shire` — update `version` and `sha256` hashes for each platform tarball

Tags containing `beta`, `alpha`, or `rc` are automatically marked as prereleases.

## Documentation

The docs site lives in `docs/src/` (mdBook). When changing user-facing behavior, update the relevant docs:

- New/changed subcommands → `CLAUDE.md` (Architecture section), `docs/src/architecture.md`, `docs/src/setup.md` (CLI reference)
- New/changed MCP tools or prompts → `docs/src/mcp-tools.md`, `CLAUDE.md` (mcp/ description)
- New/changed config options → `docs/src/configuration.md`, `CLAUDE.md` (Configuration section)
- New manifest parser → `docs/src/ecosystems.md`
- New symbol extractor → `docs/src/ecosystems.md` (Symbol extraction table)
- New reference extractor → `docs/src/ecosystems.md` (Reference extraction table)
- Changes to `shire init` behavior → `docs/src/setup.md`
- Changes to watch daemon → `docs/src/watch-daemon.md`
- Changes to worktree handling → `docs/src/worktrees.md`

## Configuration

`shire.toml` at repo root, with fallback to `~/.claude/shire.toml` if no local config exists. Key settings: `db_path`, `discovery.manifests`, `discovery.exclude`, `discovery.custom` rules, `symbols.exclude_extensions`, `symbols.exclude_patterns`, `symbols.references_enabled` (experimental), `symbols.max_file_size`, `symbols.max_references_per_file`, `docs.extensions`, `docs.max_file_size`, `watch.debounce_ms`, `log.level`, `log.dir`, `log.max_days`, `[[packages]]` overrides. Relative `db_path` values are resolved against the repo root. `SHIRE_LOG` env var overrides `log.level`.

## License

Apache-2.0
