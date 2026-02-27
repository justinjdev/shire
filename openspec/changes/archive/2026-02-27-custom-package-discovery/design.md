## Context

Shire's package discovery is driven by `ManifestParser` implementations, each keyed on a single filename. This works well for languages with standard manifests but fails for:
- Go single-module monorepos (packages defined by directory conventions, not separate `go.mod` files)
- Repos where packages are identified by a combination of files (e.g., `ownership.yml` + a build file)
- Any non-standard packaging convention

The `[[discovery.custom]]` config adds a parallel discovery mechanism that runs alongside manifest-based discovery, letting users define rules based on multi-file presence checks.

## Goals / Non-Goals

**Goals:**
- Config-driven package discovery with multi-file presence checks (AND, with glob support)
- Path scoping to limit search to specific subtrees
- Depth limits and rule-specific exclusions for performance and precision
- Discovered packages participate in symbol extraction based on their `kind`

**Non-Goals:**
- Replacing manifest-based discovery (parallel mechanism, not a replacement)
- Dependency parsing for custom-discovered packages (v1)
- OR logic in `requires` (use separate rules instead)
- Dynamic discovery rules (config only, no scripting)

## Decisions

### 1. Discovery runs as a new phase after manifest walk

Custom discovery runs after Phase 1 (manifest walk) but before Phase 3 (parse). It produces `PackageInfo` entries directly (no manifest to parse) and inserts them into the same pipeline. This means custom-discovered packages get the same treatment: symbol extraction, file indexing, source hashing.

**Why after manifest walk?** To deduplicate — if a directory is already discovered by a manifest parser, the custom rule should not create a duplicate. Last-writer-wins dedup (already in Phase 3) handles this, but skipping known directories is more efficient.

### 2. Glob matching for `requires`

Use the `glob` crate (or `globset`) to match `requires` patterns against files in each candidate directory. Patterns like `*.proto` or `Dockerfile*` are matched against filenames only (not paths). All patterns must match for the directory to qualify (AND logic).

**Why filename-only matching?** The `requires` field describes files that must exist in a directory. Path-based globs would be confusing — `paths` already handles subtree scoping.

### 3. Directory walking strategy

For each custom rule:
1. Start from each entry in `paths` (or repo root if not specified)
2. Walk directories respecting `max_depth`, global `exclude`, and rule-specific `exclude`
3. At each directory, check if ALL `requires` patterns match at least one file
4. If matched, create a `PackageInfo` with name derived from directory path + optional `name_prefix`

Reuse the existing `walkdir` infrastructure. The walk skips subdirectories of matched directories to avoid nested matches (a matched directory's children are not candidates).

**Why skip subdirectories of matches?** If `services/auth/` matches, `services/auth/internal/` should not also match — it's part of the same package.

### 4. Package naming

Package name = `{name_prefix}{relative_path}` where `relative_path` is the matched directory's path relative to repo root.

Examples with `name_prefix = "go:"`:
- `services/auth` → `go:services/auth`
- `cmd/worker` → `go:cmd/worker`

Without prefix:
- `services/auth` → `services/auth`

### 5. `kind` determines symbol extraction

The `kind` field maps to `extensions_for_kind()` in the walker — but with kind-agnostic extraction (from `add-protobuf-support`), ALL registered extensions are scanned regardless. The `kind` is still stored as the package's type for display and querying purposes.

The optional `extensions` field overrides the default extensions for the package's kind, allowing users to limit symbol extraction to specific file types for custom-discovered packages.

### 6. Config struct

```rust
#[derive(Debug, Deserialize, Default)]
pub struct CustomDiscoveryRule {
    pub name: String,
    pub kind: String,
    pub requires: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub name_prefix: Option<String>,
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
}
```

Added to `DiscoveryConfig`:
```rust
#[serde(default)]
pub custom: Vec<CustomDiscoveryRule>,
```

## Risks / Trade-offs

**[Performance] Walking large subtrees for each rule** → Mitigation: `paths` scoping limits the search space. `max_depth` prevents deep traversal. Global and rule-specific `exclude` skip irrelevant directories. Rules with no `paths` scope walk the entire repo, but that's the user's choice.

**[Deduplication] Custom rule matches directory already found by manifest parser** → Mitigation: Phase 3 dedup (last-writer-wins by path) resolves conflicts. Custom discovery runs after manifest walk, so custom rules win on conflict. This is desirable — if a user defines a custom rule, they want that rule's kind/naming to take precedence.

**[Glob matching cost] Checking `requires` patterns against directory contents** → Mitigation: `readdir` for each candidate directory is cheap compared to full file walks. Glob matching against a few filenames per directory is O(patterns × files) with small constants.

**[Nested matches] Subdirectory also matches the same rule** → Mitigation: Skip subdirectories of matched directories. First match wins (breadth-first walk).

## Open Questions

- Should custom-discovered packages participate in incremental build? Currently, incremental build keys on manifest content hashes. Custom packages have no manifest to hash. Options: (a) always rebuild custom packages, (b) hash the `requires` file contents, (c) hash the directory listing. Recommend (a) for v1 — custom discovery is cheap.
