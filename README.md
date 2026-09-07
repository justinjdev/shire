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

`shire build` walks a repository, parses manifest files, and stores packages + dependencies in a local SQLite database with full-text search. It also extracts public symbols (functions, classes, types, methods) from source files using tree-sitter, with full signatures, parameters, and return types. For 8 tier-1 languages (Go, Python, Java, TypeScript, JavaScript, Perl, Ruby, Scala), shire also extracts cross-references — calls, type references, imports, and implementations — stored in the `symbol_refs` table for call-graph and impact queries. Every file in the repo is indexed with its path, extension, size, and owning package for instant file lookup. `shire serve` exposes that index as an MCP server over stdio.

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
| `flake.nix` | nix | `inputs` attrset (dotted and block forms) |

## Install

**Homebrew** (macOS, Linux):

```sh
brew tap justinjdev/shire
brew install shire
```

**From prebuilt binary** (macOS, Linux):

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

# Initialize config
shire init              # project-level shire.toml
shire init --global     # global ~/.claude/ config for all projects

# Register with all detected AI tools (Claude Code, Cursor, VS Code, etc.)
shire install
shire install --force   # overwrite existing registrations
shire uninstall         # remove from all tools
```

### CLI reference

| Command | Flag | Description |
|---|---|---|
| `build` | `--root <DIR>` | Repository root (default: `.`) |
| | `--force` | Full rebuild, ignore cached hashes |
| | `--db <PATH>` | Database path (overrides `shire.toml`) |
| | `--config <PATH>` | Config file path (default: `<root>/shire.toml`, falls back to `~/.claude/shire.toml`) |
| `serve` | `--root <DIR>` | Repository root for on-demand reindexing (auto-rebuilds before queries) |
| | `--db <PATH>` | Database path (default: `.shire/index.db`) |
| | `--config <PATH>` | Config file path (default: `./shire.toml`, falls back to `~/.claude/shire.toml`) |
| `watch` | `--root <DIR>` | Repository root (default: `.`) |
| | `--stop` | Stop the running daemon |
| | `--status` | Print whether the daemon is running (pid, socket, liveness) and exit |
| | `--db <PATH>` | Database path (overrides `shire.toml`) |
| | `--config <PATH>` | Config file path (default: `<root>/shire.toml`, falls back to `~/.claude/shire.toml`) |
| `rebuild` | `--root <DIR>` | Repository root (default: `.`) |
| | `--file <PATH>` | Specific changed file (repeatable) |
| | `--stdin` | Read Claude Code hook JSON from stdin |
| `init` | `--root <DIR>` | Project root (default: `.`) |
| | `--global` | Set up global config in `~/.claude/` |
| | `--no-hook` | Use on-demand reindexing instead of PostToolUse hooks |
| | `-y`, `--yes` | Skip interactive prompts and use defaults |
| `install` | | Register shire as an MCP server with all detected AI tools |
| | `--dry-run` | Show what would be done without making changes |
| | `--force` | Overwrite existing registrations (useful after binary path changes) |
| `uninstall` | | Remove shire MCP registration from all detected AI tools |
| | `--dry-run` | Show what would be done without making changes |
| `clean` | `--root <DIR>` | Repository root (default: `.`) |
| | `--db <PATH>` | Database path (overrides `shire.toml`) |
| | `--config <PATH>` | Config file path (default: `<root>/shire.toml`) |

The index is written to `.shire/index.db` inside the repo root by default. You can override this with `--db` on the build command or `db_path` in `shire.toml` (see [Configuration](#configuration)). Subsequent builds are **incremental** — only manifests whose content has changed (by SHA-256 hash) are re-parsed. Source files are also tracked: if source files change without a manifest change, symbols are re-extracted automatically. An **mtime pre-check** skips SHA-256 computation entirely for packages whose source files haven't been touched since the last build. File indexing is also incremental — a file-tree hash detects structural changes, skipping Phase 9 entirely when no files have been added, removed, or resized. Symbol extraction and source hashing are **parallelized** across packages using rayon for multi-core throughput. All database writes use **batched multi-row INSERTs** within explicit transactions for maximum SQLite throughput. A per-phase **timing breakdown** is printed to stderr after each build. The server reads from this database in read-only mode.

### MCP tools

| Tool | Description |
|---|---|
| `search_packages` | Search packages by name or description |
| `list_packages` | List all indexed packages, optionally filtered by kind |
| `package_dependencies` | List a package's dependencies (set `depth>1` for transitive graph) |
| `package_dependents` | Find all packages that depend on this package |
| `search_symbols` | Search symbols by name or signature; omit query with a package filter to list all symbols in that package |
| `get_file_symbols` | List all symbols defined in a specific file |
| `search_files` | Find files by path or name (prefix match on path tokens) |
| `search_docs` | Search documentation files by content, title, or path |
| `list_package_files` | List all files in a package, optionally filtered by extension |
| `explore` | Search packages, symbols, files, and docs for a concept — returns a structured context map |
| `index_status` | Index build metadata: timestamp, git commit, counts |
| `symbol_references` | Find all references to a symbol by name (call, type, import, impl) |
| `symbol_callers` | List all callers of a function or method |
| `symbol_callees` | List what a function calls (outbound call graph) |
| `change_impact` | Blast-radius analysis: cross-references + dependency graph for a symbol (requires `symbols.references_enabled`) |
| `schema_consumers` | Find files generated from a schema file (e.g. `.proto`) |
| `generated_from` | Find the source schema file that generated a given file |

### MCP prompts

Prompts are pre-built templates for semantic codebase exploration. They compose multiple queries into structured context, giving your AI a map of where concepts live in the codebase.

| Prompt | Args | Description |
|---|---|---|
| `explore` | `query` | Search packages, symbols, files, and docs for a concept — returns a structured context map organized by package |
| `reference_audit` | `name` | Refactor-safety analysis using references, callers, and callees for change-impact review (requires cross-reference index — experimental, opt-in) |

### Claude Code

**Quick setup** — one command configures shire globally for all projects:

```sh
shire init --global
```

This creates:
- `~/.claude/shire.toml` with `db_path = "~/.claude/shire/{repo}/index.db"` (auto-namespaced per repo)
- `mcpServers.shire` entry in `~/.claude/settings.json`
- `PostToolUse` hook for auto-rebuilding the index after file edits

**On-demand mode** — skip hooks and let the MCP server rebuild automatically:

```sh
shire init --global --no-hook
```

With `--no-hook`, the MCP server starts with `--root .` and checks before each query whether the index may be stale (the Git index's mtime against the last build, resolved through the gitdir pointer in a linked worktree; "can't tell" counts as stale). If so it rebuilds — the rebuild itself compares file mtimes and SHA-256 content hashes, so it is cheap when nothing changed and correct when something did. No PostToolUse hook is installed. This is simpler but may add latency to the first query after changes.

**Per-repo setup** — for project-level config (creates `shire.toml`, `.mcp.json` with the `shire` MCP entry, and, in hook mode, a `PostToolUse` hook in `.claude/settings.json` plus `.claude/rules/shire.md`):

```sh
cd /path/to/repo
shire init          # with PostToolUse hook (default)
shire init --no-hook  # or with on-demand reindexing
shire build
```

Config is resolved with a fallback chain: `./shire.toml` → `~/.claude/shire.toml` → defaults. This means `shire build`, `shire serve`, and `shire watch` automatically pick up global config when no local config exists. Relative `db_path` values (e.g., `tmp/index.db`) are resolved against the repo root.

<details>
<summary>Manual setup</summary>

Add an `mcpServers.shire` entry to `~/.claude.json` (global) or `.mcp.json` (per-repo):

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

To keep the index fresh during a session, add a `PostToolUse` hook to `~/.claude/settings.json` (global) or `.claude/settings.json` (per-repo):

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

</details>

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

`shire watch` starts a background daemon that rebuilds the index whenever a rebuild
signal arrives — from the Claude Code `PostToolUse` hook or a manual `shire rebuild`. It
does not watch the filesystem itself; edits made outside Claude Code are only picked up
once something signals a rebuild. It uses Unix domain socket IPC with configurable
debounce (default 2s). See [Watch Daemon](docs/src/watch-daemon.md) for details,
including the `--status` flag and troubleshooting log locations.

```sh
# Start the daemon (idempotent — safe to call multiple times)
shire watch --root /path/to/repo

# Check whether it's running
shire watch --root /path/to/repo --status

# Signal a rebuild manually
shire rebuild --root /path/to/repo

# Signal a rebuild from a Claude Code hook (reads JSON from stdin; the repo root is
# resolved by walking up from cwd to the nearest shire.toml/.shire/.git)
shire rebuild --stdin

# Stop the daemon
shire watch --root /path/to/repo --stop
```

Smart filtering avoids unnecessary rebuilds: Edit/Write tools check file extension relevance and repo boundary; Bash commands are checked against an allowlist of known read-only commands (`ls`, `git status`, `cargo test`, etc.) that skip the rebuild — unknown commands default to rebuild.

## Configuration

Drop a `shire.toml` in the repo root to customize behavior. Without a local config, shire falls back to `~/.claude/shire.toml` (created by `shire init --global`). You can also point to a specific config with `--config`.

```toml
# Custom database location (default: .shire/index.db)
# Supports ~ expansion, $ENV_VARs, and {repo} (replaced with repo dir name)
db_path = "~/.claude/shire/{repo}/index.db"

[discovery]
manifests = ["package.json", "go.mod", "go.work", "Cargo.toml", "pyproject.toml", "pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "cpanfile", "Gemfile", "flake.nix"]
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

## Performance

Benchmarked on real-world monorepos (full rebuild, no incremental cache):

| Repo | Packages | Symbols | Files | Build time |
|---|---|---|---|---|
| [turborepo](https://github.com/vercel/turborepo) | 400 | 10,686 | 5,451 | ~570ms |
| [grafana](https://github.com/grafana/grafana) | 28 | 35,104 | 14,054 | ~1.1s |
| [kubernetes](https://github.com/kubernetes/kubernetes) | 34 | 78,458 | 18,275 | ~1.7s |

All queries return in **under 2ms**, most under 0.3ms. See [Performance](https://justinjdev.github.io/shire/performance.html) for detailed benchmarks and reproduction instructions.

## Architecture

Rust CLI (`main.rs`, subcommands: build, serve, watch, rebuild, init, install, uninstall, clean) dispatching into:

- **`index/`** — manifest discovery and parsing (one `ManifestParser` impl per ecosystem), incremental build orchestration
- **`symbols/`** — tree-sitter-based symbol and cross-reference extraction, plus a `cobol.rs` regex fallback
- **`db/`** — SQLite schema, FTS5 search, dependency-graph queries
- **`mcp/`** — the MCP server (17 tools, 2 prompts) served over stdio
- **`watch/`** — the Unix-only background rebuild daemon

See [Architecture](https://justinjdev.github.io/shire/architecture.html) for the full annotated source tree and the `symbol_refs` schema.

## License

Apache-2.0
