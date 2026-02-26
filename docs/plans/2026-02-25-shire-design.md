# Shire: Monorepo Package Index & MCP Server

**Date:** 2026-02-25
**Status:** Approved

## Problem

Large monorepos (1000+ packages) cause Claude Code to spend 30%+ of context on exploration before it can implement anything. Developers lose 10+ minutes per session orienting Claude to the codebase.

## Solution

`shire` (Search, Hierarchy, Index, Repo Explorer) is a Rust CLI that:
1. Scans a monorepo for package manifests (package.json, go.mod, Cargo.toml, pyproject.toml)
2. Builds a SQLite index of all packages and their dependency graph
3. Serves the index as an MCP server so Claude can query it directly

## Architecture

Single Rust binary, two modes:

```
shire build [--root <path>]    # scan repo, produce index
shire serve                     # MCP server over stdio
```

### Project Structure

```
shire/
├── src/
│   ├── main.rs              # CLI entry (clap)
│   ├── index/
│   │   ├── mod.rs           # orchestrator: walk repo, dispatch to parsers
│   │   ├── manifest.rs      # trait ManifestParser + implementations
│   │   ├── npm.rs           # package.json parser
│   │   ├── go.rs            # go.mod parser
│   │   ├── cargo.rs         # Cargo.toml parser
│   │   └── python.rs        # pyproject.toml / setup.py parser
│   ├── db/
│   │   ├── mod.rs           # schema creation, migrations
│   │   └── queries.rs       # typed query functions
│   ├── mcp/
│   │   ├── mod.rs           # MCP server setup (rmcp)
│   │   └── tools.rs         # tool definitions + handlers
│   └── config.rs            # shire.toml parsing
├── Cargo.toml
└── shire.toml.example
```

### Dependencies

- `rusqlite` (bundled SQLite + FTS5)
- `rmcp` (MCP protocol, stdio transport)
- `clap` (CLI)
- `serde` / `serde_json` (manifest parsing)
- `toml` (Cargo.toml + config parsing)
- `walkdir` (directory traversal)
- `ignore` (respects .gitignore)

## Database Schema

```sql
CREATE TABLE packages (
    name        TEXT PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    kind        TEXT NOT NULL,         -- 'npm', 'go', 'cargo', 'python'
    version     TEXT,
    description TEXT,
    metadata    TEXT                   -- JSON blob for extensibility
);

CREATE TABLE dependencies (
    package     TEXT NOT NULL REFERENCES packages(name),
    dependency  TEXT NOT NULL,
    dep_kind    TEXT NOT NULL DEFAULT 'runtime',
    version_req TEXT,
    is_internal INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (package, dependency, dep_kind)
);

CREATE VIRTUAL TABLE packages_fts USING fts5(
    name, description, path,
    content='packages',
    content_rowid='rowid'
);

CREATE TABLE shire_meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);
```

## MCP Tools

| Tool | Purpose | Key params |
|------|---------|------------|
| `search_packages` | Fuzzy search packages by name/description | `query` |
| `get_package` | Full details for one package | `name` |
| `package_dependencies` | What does this package depend on? | `name`, `internal_only` |
| `package_dependents` | What depends on this package? | `name` |
| `dependency_graph` | Transitive dep graph from a root | `name`, `depth`, `internal_only` |
| `list_packages` | List all packages, optionally by kind | `kind` |
| `index_status` | Index freshness and stats | (none) |

## Configuration

### `shire.toml` (optional, repo root)

```toml
[discovery]
manifests = ["package.json", "go.mod", "Cargo.toml", "pyproject.toml"]
exclude = ["vendor", "node_modules", "dist", ".build", "third_party"]

[[packages]]
name = "legacy-auth"
description = "Legacy auth service - deprecated, use auth-service instead"
tags = ["deprecated"]
```

### Index Location

Default: `.shire/index.db` in repo root. Override via `SHIRE_DB_PATH` env var.

### Claude Code Integration

```json
{
  "mcpServers": {
    "shire": {
      "command": "shire",
      "args": ["serve"]
    }
  }
}
```

## Distribution

```bash
cargo install shire
cd /path/to/monorepo
shire build
claude mcp add shire -- shire serve
```

## Future Additions (not in v1)

- File-level indexing (path, package, kind classification)
- Symbol indexing via tree-sitter (functions, types, classes)
- `shire build --incremental` (re-index only changed files)
- HTTP/SSE transport for `shire serve`
- CI-built index artifact distribution
