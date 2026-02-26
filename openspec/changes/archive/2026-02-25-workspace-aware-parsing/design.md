# Workspace-Aware Parsing — Design

## Context

The current indexer parses each manifest file independently. This works for self-contained manifests but breaks when ecosystems use workspace features that establish parent-child relationships between manifests.

## Goals / Non-Goals

**Goals:**
- Resolve Cargo `workspace = true` dependencies to actual versions
- Normalize npm `workspace:` protocol versions
- Parse `go.work` files and annotate Go packages with workspace membership

**Non-Goals:**
- Full workspace resolution (lock files, transitive workspace deps)
- Modifying DB schema (metadata field handles `go.work` context)
- Supporting Python workspaces (no standard protocol exists)

## Decisions

### Decision 1: Two-pass architecture for Cargo

The walk phase already discovers all `Cargo.toml` files. Before parsing members, we scan for workspace roots (those with `[workspace]` section) and build a map of `dep_name → version_req` from `[workspace.dependencies]`.

**How it works:**
1. During the parse phase, before dispatching to `CargoParser`, pre-scan all Cargo.toml files for `[workspace]` sections
2. Build a `HashMap<String, String>` of workspace dep name → version
3. Pass this context to `CargoParser::parse()` via an extended signature

**Why not a separate pre-pass in `build_index`?** The walk phase already collects all manifest paths. We can do the workspace scan lazily during the parse phase by reading the Cargo.toml files that have `[workspace]` but no `[package]`. These already fail to parse (returning an error), so we'd just extract context from them before moving on.

**Implementation:** Add an optional `workspace_deps: Option<&HashMap<String, String>>` parameter to `CargoParser`. The orchestrator collects workspace deps from root Cargo.tomls and passes them through.

### Decision 2: Simple prefix strip for npm

npm workspace versions use the format `workspace:*`, `workspace:^`, `workspace:~1.0.0`. The fix is a simple string operation in `extract_deps` — if the version starts with `workspace:`, strip that prefix.

No workspace context needed from root `package.json` — the dep name matching already handles internal detection correctly.

### Decision 3: go.work as metadata-only context

`go.work` files don't contain packages or dependencies themselves. They list `use` directives pointing to member directories. We:

1. Add `go.work` to default discoverable manifests
2. Create a `GoWorkParser` that returns an error (not a package) but extracts member dirs
3. In the orchestrator, before parsing Go modules, scan for `go.work` files and build a set of workspace member directories
4. Pass this set when parsing `go.mod` — if the module's directory matches a `use` directive, set `metadata: {"go_workspace": true}`

**Why metadata instead of a new field?** The `metadata` JSON field already exists on `PackageInfo` and is stored in the DB. No schema changes needed.

### Decision 4: Parser trait stays unchanged

Rather than modifying the `ManifestParser` trait signature (which would force changes to all parsers), the orchestrator handles workspace context collection and passes it to specific parsers via their concrete types. The trait's `parse()` method stays `(&self, &Path, &str) -> Result<PackageInfo>`.

Cargo and Go parsers get new methods (e.g., `parse_with_context()`) that the orchestrator calls directly when it has workspace context available.

## Approach

1. **Cargo workspace collection:** In `build_index`, after walking manifests, iterate Cargo.toml files to find workspace roots. Extract `[workspace.dependencies]` into a map.
2. **Cargo parser update:** Add `parse_with_workspace_deps()` method that accepts the deps map. When a dep has `workspace = true`, look up the version from the map.
3. **npm strip:** In `npm.rs`, strip `workspace:` prefix from version strings during dep extraction.
4. **go.work parsing:** New `go_work.rs` module that parses `go.work` `use` directives. The orchestrator calls it during the pre-parse scan.
5. **Go metadata:** In the orchestrator, when parsing `go.mod` files, check if the relative_dir matches a `go.work` use directive. If so, set metadata.
6. **Config update:** Add `go.work` to `default_manifests()`.
