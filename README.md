# shire

**S**earch, **H**ierarchy, **I**ndex, **R**epo **E**xplorer — a monorepo package indexer that builds a dependency graph in SQLite and serves it over [Model Context Protocol](https://modelcontextprotocol.io/).

Point it at a monorepo. It discovers every package, maps their dependency relationships, and gives your AI tools structured access to the result.

## What it does

`shire build` walks a repository, parses manifest files, and stores packages + dependencies in a local SQLite database with full-text search. It also extracts public symbols (functions, classes, types, methods) from source files using tree-sitter, with full signatures, parameters, and return types. Every file in the repo is indexed with its path, extension, size, and owning package for instant file lookup. `shire serve` exposes that index as an MCP server over stdio.

**Supported ecosystems:**

| Manifest | Kind | Workspace support |
|---|---|---|
| `package.json` | npm | `workspace:` protocol versions normalized |
| `go.mod` | go | `go.work` member metadata |
| `go.work` | go | `use` directives parsed for workspace context |
| `Cargo.toml` | cargo | `workspace = true` deps resolved from root |
| `pyproject.toml` | python | — |
| `pom.xml` | maven | Parent POM inheritance (groupId, version) |
| `build.gradle` / `build.gradle.kts` | gradle | `settings.gradle` project inclusion |

## Install

```sh
cargo install --path .
```

## Usage

```sh
# Index a monorepo
shire build --root /path/to/repo

# Rebuild from scratch (ignore cached hashes)
shire build --root /path/to/repo --force

# Start the MCP server
shire serve
```

The index is written to `.shire/index.db` inside the repo root. Subsequent builds are **incremental** — only manifests whose content has changed (by SHA-256 hash) are re-parsed. Source files are also tracked: if source files change without a manifest change, symbols are re-extracted automatically. The server reads from this database in read-only mode.

### MCP tools

| Tool | Description |
|---|---|
| `search_packages` | Full-text search across package names, descriptions, and paths |
| `get_package` | Exact name lookup for a single package |
| `list_packages` | List all packages, optionally filtered by kind |
| `package_dependencies` | What a package depends on (optionally internal-only) |
| `package_dependents` | Reverse lookup — what depends on this package |
| `dependency_graph` | Transitive BFS traversal from a root package |
| `search_symbols` | Full-text search across symbol names and signatures |
| `get_package_symbols` | List all symbols in a package (functions, classes, types, methods) |
| `get_symbol` | Exact name lookup for a symbol across packages |
| `search_files` | Full-text search across file paths, with optional package/extension filter |
| `list_package_files` | List all files belonging to a package, with optional extension filter |
| `index_status` | When the index was built, git commit, package count |

### Claude Desktop

Add to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "shire": {
      "command": "shire",
      "args": ["serve", "--db", "/path/to/repo/.shire/index.db"]
    }
  }
}
```

## Configuration

Drop a `shire.toml` in the repo root to customize discovery:

```toml
[discovery]
manifests = ["package.json", "go.mod", "go.work", "Cargo.toml", "pyproject.toml", "pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"]
exclude = ["node_modules", "vendor", "dist", ".build", "target", "third_party", ".shire"]

# Override package descriptions
[[packages]]
name = "legacy-auth"
description = "Deprecated auth service — do not add new dependencies"
```

All fields are optional. Defaults are shown above.

## Architecture

```
src/
├── main.rs          # CLI (clap): build and serve subcommands
├── config.rs        # shire.toml parsing
├── db/
│   ├── mod.rs       # SQLite schema, open/create
│   └── queries.rs   # FTS search, dependency graph BFS, listing
├── index/
│   ├── mod.rs       # Walk + incremental index orchestrator
│   ├── manifest.rs  # ManifestParser trait
│   ├── hash.rs      # SHA-256 content hashing for incremental builds
│   ├── npm.rs       # package.json parser (workspace: protocol)
│   ├── go.rs        # go.mod parser
│   ├── go_work.rs   # go.work parser (workspace use directives)
│   ├── cargo.rs     # Cargo.toml parser (workspace dep resolution)
│   ├── python.rs    # pyproject.toml parser
│   ├── maven.rs     # pom.xml parser (parent POM inheritance)
│   ├── gradle.rs    # build.gradle / build.gradle.kts parser
│   └── gradle_settings.rs # settings.gradle parser (project inclusion)
├── symbols/
│   ├── mod.rs       # Symbol types, orchestrator (dispatch by package kind)
│   ├── walker.rs    # Source file discovery (extension filtering, excludes)
│   ├── typescript.rs # TS/JS extractor (tree-sitter)
│   ├── go.rs        # Go extractor (tree-sitter)
│   ├── rust_lang.rs # Rust extractor (tree-sitter)
│   └── python.rs    # Python extractor (tree-sitter)
└── mcp/
    ├── mod.rs       # MCP server setup (rmcp, stdio transport)
    └── tools.rs     # 12 tool handlers
```

## License

MIT
