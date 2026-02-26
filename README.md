# shire

**S**earch, **H**ierarchy, **I**ndex, **R**epo **E**xplorer — a monorepo package indexer that builds a dependency graph in SQLite and serves it over [Model Context Protocol](https://modelcontextprotocol.io/).

Point it at a monorepo. It discovers every package, maps their dependency relationships, and gives your AI tools structured access to the result.

## What it does

`shire build` walks a repository, parses manifest files, and stores packages + dependencies in a local SQLite database with full-text search. `shire serve` exposes that index as an MCP server over stdio.

**Supported ecosystems:**

| Manifest | Kind |
|---|---|
| `package.json` | npm |
| `go.mod` | go |
| `Cargo.toml` | cargo |
| `pyproject.toml` | python |

## Install

```sh
cargo install --path .
```

## Usage

```sh
# Index a monorepo
shire build --root /path/to/repo

# Start the MCP server
shire serve
```

The index is written to `.shire/index.db` inside the repo root. The server reads from this database in read-only mode.

### MCP tools

| Tool | Description |
|---|---|
| `search_packages` | Full-text search across package names, descriptions, and paths |
| `get_package` | Exact name lookup for a single package |
| `list_packages` | List all packages, optionally filtered by kind |
| `package_dependencies` | What a package depends on (optionally internal-only) |
| `package_dependents` | Reverse lookup — what depends on this package |
| `dependency_graph` | Transitive BFS traversal from a root package |
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
manifests = ["package.json", "go.mod", "Cargo.toml", "pyproject.toml"]
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
│   ├── mod.rs       # Walk + index orchestrator
│   ├── manifest.rs  # ManifestParser trait
│   ├── npm.rs       # package.json parser
│   ├── go.rs        # go.mod parser
│   ├── cargo.rs     # Cargo.toml parser
│   └── python.rs    # pyproject.toml parser
└── mcp/
    ├── mod.rs       # MCP server setup (rmcp, stdio transport)
    └── tools.rs     # 7 tool handlers
```

## License

MIT
