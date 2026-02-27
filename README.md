# shire

<div align="center">
<pre>
                       .,:lccc:,.
                  .,codxkkOOOOkkxdoc,.
              .;ldkkOOOOOOOOOOOOOOOkkdl;.
           .:oxOOkxdollccccccccllodxkOOkxo:.
         ,lkOOxl;..                ..,lxOOkl,
       .ckOOd:.                        .:dOOkc.
      ;xOOo,          .,clllc,.          ,oOOx;
     lOOk;         .:dkOOOOOOkd:.         ;kOOl
    oOOx,        .ckOOOOOOOOOOOOkc.        ,xOOo
   lOOk,        ;xOOOkdl:;;:ldkOOOx;        ,kOOl
  ;OOO;        lOOOd;.        .;dOOOl        ;OOO;
  dOOd        :OOOl              lOOO:        dOOd
  kOOl        oOOx      .;;.     xOOo        lOOk
  kOOl        oOOx     .xOOx.    xOOo        lOOk
  dOOd        :OOOl    .oOOo.   lOOO:        dOOd
  ;OOO;        lOOOd;.  .,,. .;dOOOl        ;OOO;
   lOOk,        ;xOOOkdl:,:ldkOOOx;        ,kOOl
    oOOx,        .ckOOOOOOOOOOOOkc.        ,xOOo
     lOOk;         .:dkOOOOOOkd:.         ;kOOl
      ;xOOo,          .,clllc,.          ,oOOx;
       .ckOOd:.                        .:dOOkc.
         ,lkOOxl;..                ..,lxOOkl,
           .:oxOOkxdollccccccccllodxkOOkxo:.
              .;ldkkOOOOOOOOOOOOOOOkkdl;.
                  .,codxkkOOOOkkxdoc,.
                       .,:lccc:,.
</pre>
</div>

*One index to rule them all.*

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
| `cpanfile` | perl | `requires` / `on 'test'` blocks |
| `Gemfile` | ruby | `gem` / `group :test` blocks |

## Install

**From prebuilt binary** (macOS, Linux, Windows):

Download the latest release from [GitHub Releases](https://github.com/justinjdev/shire/releases) and add to your PATH.

**From source:**

```sh
cargo install --path .
```

## Usage

```sh
# Index a monorepo
shire build --root /path/to/repo

# Rebuild from scratch (ignore cached hashes)
shire build --root /path/to/repo --force

# Write the index to a custom location
shire build --root /path/to/repo --db /tmp/my-index.db

# Start the MCP server
shire serve

# Auto-rebuild: start watch daemon, then stop it
shire watch --root /path/to/repo
shire watch --root /path/to/repo --stop

# Signal a rebuild (from a hook or manually)
shire rebuild --root /path/to/repo
```

The index is written to `.shire/index.db` inside the repo root by default. You can override this with `--db` on the build command or `db_path` in `shire.toml` (see [Configuration](#configuration)). Subsequent builds are **incremental** — only manifests whose content has changed (by SHA-256 hash) are re-parsed. Source files are also tracked: if source files change without a manifest change, symbols are re-extracted automatically. An **mtime pre-check** skips SHA-256 computation entirely for packages whose source files haven't been touched since the last build. File indexing is also incremental — a file-tree hash detects structural changes, skipping Phase 9 entirely when no files have been added, removed, or resized. Symbol extraction and source hashing are **parallelized** across packages using rayon for multi-core throughput. All database writes use **batched multi-row INSERTs** within explicit transactions for maximum SQLite throughput. A per-phase **timing breakdown** is printed to stderr after each build. The server reads from this database in read-only mode.

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
| `get_file_symbols` | List all symbols defined in a specific file |
| `search_files` | Full-text search across file paths, with optional package/extension filter |
| `list_package_files` | List all files belonging to a package, with optional extension filter |
| `index_status` | When the index was built, git commit, package/symbol/file counts, build duration |

### MCP prompts

Prompts are pre-built templates for semantic codebase exploration. They compose multiple queries into structured context, giving your AI a map of where concepts live in the codebase.

| Prompt | Args | Description |
|---|---|---|
| `explore` | `query` | Search packages, symbols, and files for a concept — returns a structured context map organized by package |
| `explore-package` | `name` | Deep dive into a specific package — metadata, internal deps, dependents, public API surface, file tree |
| `explore-area` | `path` | Explore a directory subtree — packages, files, and symbol summaries under a path prefix |
| `onboard` | — | Repository overview for onboarding — tech stack, package counts by language, file distribution, index freshness |
| `impact-analysis` | `name` | Blast radius analysis — direct dependents, transitive dependents, full dependency chain |
| `understand-dependency` | `from`, `to` | Trace the dependency path between two packages |

### Claude Code

Add to your project's `.claude/settings.json`:

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

Or add globally in `~/.claude/settings.json` to use across all projects.

To keep the index fresh during a session, add a `PostToolUse` hook that signals the watch daemon after file-modifying tools:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write|NotebookEdit|Bash",
        "hooks": [{ "type": "command", "command": "shire rebuild --stdin" }]
      }
    ]
  }
}
```

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

### Watch daemon

`shire watch` starts a background daemon that auto-rebuilds the index when files change. It uses Unix domain socket IPC with configurable debounce (default 2s).

```sh
# Start the daemon (idempotent — safe to call multiple times)
shire watch --root /path/to/repo

# Signal a rebuild manually
shire rebuild --root /path/to/repo

# Signal a rebuild from a Claude Code hook (reads JSON from stdin, uses cwd as repo root)
shire rebuild --stdin

# Stop the daemon
shire watch --root /path/to/repo --stop
```

Smart filtering avoids unnecessary rebuilds: Edit/Write tools check file extension relevance and repo boundary; Bash commands are filtered against a denylist of known read-only commands (`ls`, `git status`, `cargo test`, etc.) — unknown commands default to rebuild.

## Configuration

Drop a `shire.toml` in the repo root to customize behavior:

```toml
# Custom database location (default: .shire/index.db)
db_path = "/path/to/custom/index.db"

[discovery]
manifests = ["package.json", "go.mod", "go.work", "Cargo.toml", "pyproject.toml", "pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "cpanfile", "Gemfile"]
exclude = ["node_modules", "vendor", "dist", ".build", "target", "third_party", ".shire", ".gradle", "build"]

# Skip symbol extraction for specific file types
[symbols]
exclude_extensions = [".proto", ".pl"]

# Override package descriptions
[[packages]]
name = "legacy-auth"
description = "Deprecated auth service — do not add new dependencies"
```

All fields are optional. Defaults are shown above. The `--db` CLI flag takes precedence over `db_path` in config.

### Custom package discovery

For codebases where packages aren't defined by standard manifest files — Go single-module monorepos, repos that use `ownership.yml` + build files, or any non-standard convention — you can define custom discovery rules:

```toml
# Discover Go apps: directories containing both main.go and ownership.yml
[[discovery.custom]]
name = "go-apps"
kind = "go"
requires = ["main.go", "ownership.yml"]
paths = ["services/", "cmd/"]
exclude = ["testdata", "examples"]
max_depth = 3
name_prefix = "go:"

# Discover proto packages: directories containing *.proto and buf.yaml
[[discovery.custom]]
name = "proto-packages"
kind = "proto"
requires = ["*.proto", "buf.yaml"]
paths = ["proto/", "services/"]
max_depth = 4
```

| Field | Required | Description |
|---|---|---|
| `name` | yes | Rule identifier |
| `kind` | yes | Package kind for symbol extraction (`go`, `proto`, `npm`, etc.) |
| `requires` | yes | File patterns that must ALL exist in a directory (supports globs like `*.proto`) |
| `paths` | no | Limit search to specific subtrees (default: repo root) |
| `exclude` | no | Rule-specific directory exclusions (on top of global excludes) |
| `max_depth` | no | Maximum depth to search from each `paths` entry |
| `name_prefix` | no | Prefix prepended to directory-derived package name (e.g., `go:services/auth`) |
| `extensions` | no | Override which file extensions get symbol extraction |

Custom discovery runs alongside manifest-based discovery. Directories already found by manifest parsers are skipped. Subdirectories of matched directories are also skipped to prevent nested matches.

## Architecture

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

## License

MIT
