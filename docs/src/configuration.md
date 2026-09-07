# Configuration

Drop a `shire.toml` in the repo root to customize behavior:

```toml
# Custom database location (default: .shire/index.db)
db_path = "/path/to/custom/index.db"

[discovery]
manifests = ["package.json", "go.mod", "go.work", "Cargo.toml", "pyproject.toml", "pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "cpanfile", "Gemfile", "flake.nix"]
exclude = ["node_modules", "vendor", "dist", ".build", "target", "third_party", ".shire", ".gradle", "build"]

# Symbol extraction
[symbols]
exclude_extensions = [".proto", ".pl"]
exclude_patterns = []       # file name patterns to skip (suffix match, e.g. "_generated.go"; or prefix, e.g. "zz_generated.")
references_enabled = false  # EXPERIMENTAL, default false — see below
max_file_size = 0           # 0 = disabled (default); set to e.g. 2097152 for 2 MiB cap
max_references_per_file = 10000  # 0 = unlimited; default 10000 — caps cross-references per file

# Documentation indexing
[docs]
extensions = [".md", ".rst", ".txt", ".adoc"]
max_file_size = 262144  # 256 KB — files larger than this are truncated

# MCP server on-demand rebuild
[serve]
debounce_s = 5  # minimum seconds between rebuild checks during MCP tool call bursts

# Override package descriptions
[[packages]]
name = "legacy-auth"
description = "Deprecated auth service — do not add new dependencies"
```

## Config precedence

Config is resolved in this order, with **no merging** — the first one found is used whole, and none of the others are read:

1. `--config <PATH>` — explicit path, must exist
2. `./shire.toml` — repo-root config
3. `~/.claude/shire.toml` — global config (created by `shire init --global`)
4. Built-in defaults

Because the fallback is whole-file replacement rather than a merge, a local `shire.toml` containing only `db_path` discards every other setting in `~/.claude/shire.toml` (excludes, custom discovery rules, etc.) rather than layering on top of it.

## Watch daemon

```toml
[watch]
debounce_ms = 2000  # milliseconds to wait after last change before rebuilding
```

## Logging

```toml
[log]
level = "warn"          # error, warn, info, debug, trace
dir = ".shire/logs"     # log directory (relative to repo root). Set to "" to disable file logging
max_days = 30           # automatically delete log files older than this
```

The `SHIRE_LOG` environment variable overrides the config `level` (e.g., `SHIRE_LOG=debug shire build`). Log files are daily-rotated with filenames like `shire.log.2026-03-26`. Each session includes a unique session ID for correlation across concurrent processes.

All fields are optional. Defaults are shown above. The `--db` CLI flag takes precedence over `db_path` in config.

## Cross-reference index (experimental)

`symbols.references_enabled` (default `false`) populates the `symbol_refs`
table so the `symbol_references`, `symbol_callers`, and `symbol_callees`
MCP tools can answer "where is this used?" / "who calls this?" questions.
Reference extraction is supported for 8 tier-1 languages: Go, Python,
Java, TypeScript, JavaScript, Perl, Ruby, Scala.

**Opt-in:** `shire init` asks whether to enable this (prompt labelled
experimental), and writes `references_enabled = true` to `shire.toml`
when you say yes. You can also add it manually:

```toml
[symbols]
references_enabled = true
```

**Cost:** DB grows substantially — roughly +30% on TS/JS repos to +150% on
Go-heavy repos (benchmarks on shire-bench: turborepo +29%, grafana +152%,
kubernetes +104% vs main baseline). Build time grows ~5-7%.

Toggling the flag takes effect on the next build. Disabling wipes
`symbol_refs` at the start of the build; re-enabling repopulates it on
the next full rebuild (`shire build --force`).

This feature is marked experimental: its schema and coverage may change
in minor versions as language support broadens and edge cases surface.

## Custom package discovery

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
