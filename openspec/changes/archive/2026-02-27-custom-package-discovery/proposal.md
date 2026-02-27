## Why

Shire's package discovery is hardcoded to a fixed set of manifest filenames (`package.json`, `go.mod`, `Cargo.toml`, etc.). This fails for codebases where packages are defined by conventions that don't map to a single manifest file — most notably Go single-module monorepos (0 packages indexed despite thousands of `.go` files), but also any repo where packages are identified by a combination of files (e.g., `ownership.yml` + a build file). Users need a way to define custom discovery rules without modifying Shire's source.

## What Changes

- Add a `[[discovery.custom]]` config section in `shire.toml` for user-defined package discovery rules
- Each rule defines:
  - **`name`** — rule identifier
  - **`kind`** — which symbol extractor to use (e.g., `go`, `proto`, `gradle`)
  - **`requires`** — file patterns that must ALL be present in a directory (supports globs like `*.proto`)
  - **`paths`** — scope search to specific subtrees (default: repo root)
  - **`exclude`** — rule-specific directory exclusions (on top of global excludes)
  - **`max_depth`** — how deep from `paths` roots to search
  - **`name_prefix`** — optional prefix prepended to directory-derived package name
  - **`extensions`** — override which file extensions get symbol extraction (defaults based on `kind`)
- Discovered packages get their name from the directory path relative to repo root (with optional prefix)
- Symbol extraction runs based on the `kind` field, using existing tree-sitter extractors
- Custom discovery runs as a parallel mechanism alongside existing manifest-based discovery — does not replace or filter it
- No dependency parsing for custom-discovered packages in v1

Example config:
```toml
[[discovery.custom]]
name = "go-apps"
kind = "go"
requires = ["main.go", "ownership.yml"]
paths = ["services/", "cmd/"]
exclude = ["testdata", "examples"]
max_depth = 3
name_prefix = "go:"

[[discovery.custom]]
name = "proto-packages"
kind = "proto"
requires = ["*.proto", "buf.yaml"]
paths = ["proto/", "services/"]
max_depth = 4
```

## Capabilities

### New Capabilities
- `custom-package-discovery`: Config-driven package discovery using multi-file presence checks with glob support, path scoping, depth limits, and rule-specific exclusions, running parallel to existing manifest-based discovery

### Modified Capabilities
- `configuration`: Add `[[discovery.custom]]` table array with `name`, `kind`, `requires`, `paths`, `exclude`, `max_depth`, `name_prefix`, and `extensions` fields
- `package-discovery`: Run custom discovery rules after manifest walk, register matched directories as packages
- `symbol-extraction`: Ensure custom-discovered packages get symbol extraction based on their `kind` and optional `extensions` override

## Impact

- **Code**: `src/config.rs` (new config structs), `src/index/mod.rs` (pipeline integration — new discovery phase), `src/symbols/walker.rs` (kind-to-extension mapping for custom packages)
- **Behavior**: No change to default behavior. Only active when `[[discovery.custom]]` rules are configured.
- **DB**: New package rows for custom-discovered packages; no schema changes
- **Dependencies**: `glob` or similar crate for file pattern matching in `requires`
- **Supersedes**: `fix-go-single-module` — Go app discovery becomes a config-driven use case rather than a built-in feature
