# Architecture

```
src/
├── main.rs          # CLI (clap): build, serve, watch, rebuild, init, clean subcommands
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
│   ├── perl.rs      # Perl extractor (regex-based)
│   ├── ruby.rs      # Ruby extractor (regex-based)
│   └── elixir.rs    # Elixir extractor (regex-based)
├── rag/             # Optional RAG vector search (behind `rag` feature flag)
│   ├── mod.rs       # Feature-gated module root
│   ├── embedder.rs  # fastembed wrapper, symbol text formatting, batch embedding
│   └── storage.rs   # sqlite-vec extension, vec0 table, vector CRUD, KNN search
├── mcp/
│   ├── mod.rs       # MCP server setup (rmcp, stdio transport)
│   ├── tools.rs     # 10 tool handlers (+ hybrid search when RAG enabled)
│   └── prompts.rs   # explore prompt template for semantic codebase exploration
└── watch/
    ├── mod.rs       # Daemon event loop (UDS listener, debounce, rebuild)
    ├── daemon.rs    # Process management (start/stop/is_running via PID)
    └── protocol.rs  # Hook input parsing, Bash read-only denylist
```
