# Architecture

```
src/
├── main.rs          # CLI (clap): build, serve, watch, rebuild subcommands
├── config.rs        # shire.toml parsing
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
│   ├── typescript.rs # TS/JS extractor (tree-sitter)
│   ├── go.rs        # Go extractor (tree-sitter)
│   ├── rust_lang.rs # Rust extractor (tree-sitter)
│   ├── python.rs    # Python extractor (tree-sitter)
│   ├── proto.rs     # Protobuf extractor (tree-sitter)
│   ├── java.rs      # Java extractor (tree-sitter)
│   ├── kotlin.rs    # Kotlin extractor (tree-sitter)
│   ├── perl.rs      # Perl extractor (regex-based)
│   └── ruby.rs      # Ruby extractor (tree-sitter)
├── mcp/
│   ├── mod.rs       # MCP server setup (rmcp, stdio transport)
│   ├── tools.rs     # 13 tool handlers
│   └── prompts.rs   # 6 prompt templates for semantic codebase exploration
└── watch/
    ├── mod.rs       # Daemon event loop (UDS listener, debounce, rebuild)
    ├── daemon.rs    # Process management (start/stop/is_running via PID)
    └── protocol.rs  # Hook input parsing, Bash read-only denylist
```
