# Architecture

```
src/
├── main.rs          # CLI (clap): build, serve, watch, rebuild, init, clean subcommands
├── lib.rs           # Library re-exports for embedding shire as a crate
├── config.rs        # shire.toml parsing
├── git.rs           # Git worktree detection and repo root resolution
├── init.rs          # `shire init` setup (config, MCP server, hooks, rules)
├── db/
│   ├── mod.rs       # SQLite schema, open/create
│   └── queries.rs   # FTS search, dependency graph BFS, listing
├── index/
│   ├── mod.rs       # Walk + incremental index orchestrator
│   ├── custom_discovery.rs # Config-driven custom package discovery
│   ├── manifest.rs  # ManifestParser trait
│   ├── hash.rs      # SHA-256 content hashing for incremental builds
│   ├── npm.rs       # package.json parser (workspace: protocol)
│   ├── go.rs        # go.mod parser
│   ├── go_work.rs   # go.work parser (workspace use directives)
│   ├── cargo.rs     # Cargo.toml parser (workspace dep resolution)
│   ├── python.rs    # pyproject.toml parser
│   ├── maven.rs     # pom.xml parser (parent POM inheritance)
│   ├── gradle.rs    # build.gradle / build.gradle.kts parser
│   ├── gradle_settings.rs # settings.gradle parser (project inclusion)
│   ├── perl.rs      # cpanfile parser (requires, on 'test')
│   └── ruby.rs      # Gemfile parser (gem, group blocks)
├── symbols/
│   ├── mod.rs       # Symbol types, kind-agnostic extraction orchestrator
│   ├── walker.rs    # Source file discovery (extension filtering, excludes)
│   ├── registry.rs  # Language registry: maps extensions to tree-sitter grammars + hooks
│   ├── query_extract.rs # Generic tree-sitter query executor with hook callbacks
│   ├── queries/     # Tree-sitter .scm query files (one per language)
│   ├── hooks/       # Language-specific hooks (visibility, signatures, params, post-processing)
│   ├── elixir.rs    # Elixir extractor (regex-based)
│   └── cobol.rs     # COBOL extractor (regex-based)
│                    # Cross-reference extraction (call, type, import, impl) is supported
│                    # for 8 tier-1 languages: Go, Python, Java, TypeScript, JavaScript,
│                    # Perl, Ruby, Scala. References are captured via @reference.* captures
│                    # in the language's .scm query and written to the symbol_refs table.
│                    # Coverage is asymmetric per language: JavaScript omits Type refs
│                    # (no type system), and Go/Perl omit Impl refs (no extends/implements).
├── rag/             # Optional RAG vector search (behind `rag` feature flag)
│   ├── mod.rs       # Feature-gated module root
│   ├── embedder.rs  # fastembed wrapper, file-level text formatting, batch embedding (64 files/batch)
│   └── storage.rs   # sqlite-vec extension, vec0 table, vector CRUD, KNN search
├── mcp/
│   ├── mod.rs       # MCP server setup (rmcp, stdio transport)
│   ├── tools.rs     # 11 tool handlers (+ hybrid search when RAG enabled)
│   └── prompts.rs   # explore prompt template for semantic codebase exploration
└── watch/
    ├── mod.rs       # Daemon event loop (UDS listener, debounce, rebuild)
    ├── daemon.rs    # Process management (start/stop/is_running via PID)
    └── protocol.rs  # Hook input parsing, Bash read-only denylist
```

## symbol_refs table

The `symbol_refs` table stores cross-reference records extracted alongside symbol definitions. Each row captures a reference to a named symbol:

| Column | Type | Description |
|---|---|---|
| `name` | TEXT | The name being referenced (function, type, module, etc.) |
| `kind` | TEXT | One of: `call`, `type`, `import`, `impl` |
| `file_path` | TEXT | Source file containing the reference |
| `line` | INTEGER | Line number of the reference |
| `package` | TEXT | Package the referencing file belongs to (nullable) |
| `enclosing_symbol` | TEXT | Nearest enclosing function or method (nullable) |

B-tree indexes on `name`, `file_path`, and `enclosing_symbol` support the exact-match lookups used by the `symbol_references`, `symbol_callers`, and `symbol_callees` MCP tools. No FTS5 table — reference queries are exact-name only.

Incremental behavior mirrors symbol extraction: references for a file are dropped and re-extracted whenever the file's SHA-256 hash changes. No separate pass is needed — references are extracted in the same tree-sitter walk as symbol definitions.

## File embeddings

When [RAG is enabled](./configuration.md#rag-vector-search), the build produces file-level vector embeddings for hybrid search. Each file is represented as a `FileForEmbedding` containing its symbols via `FileSymbol` (name, kind, and optional signature).

The text representation (`file_to_text`) works as follows:
- Symbols are sorted by kind then name
- Signatures are preferred over `kind name` fallback when available
- A **total character budget of 1800** caps the output text — after the file path prefix is accounted for, remaining budget is filled with symbols until exhausted
- Files with no symbols produce a minimal `file <path> in <package>` string

Embedding runs in a **background thread** spawned during the build, executing concurrently with post-build housekeeping:
- Files are processed in **batches of 64** to balance throughput and memory
- A progress callback reports batch completion for progress bar updates
- Errors (model init, embedding, DB write) are reported on the progress bar rather than failing the build
