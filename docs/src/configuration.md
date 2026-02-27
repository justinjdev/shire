# Configuration

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
