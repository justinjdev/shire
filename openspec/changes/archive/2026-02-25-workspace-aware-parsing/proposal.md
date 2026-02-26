# Workspace-Aware Parsing

## Why

Monorepos frequently use workspace features — npm workspaces, Cargo workspaces, Go workspaces. Currently shire parses each manifest in isolation, which causes:

1. **Cargo**: Members using `dep = { workspace = true }` get `version_req: None` because the workspace root's `[workspace.dependencies]` isn't read.
2. **npm**: Dependencies with `workspace:*` or `workspace:^` version protocols are stored verbatim instead of being normalized.
3. **Go**: `go.work` files are ignored entirely. While `go.mod` files are discovered independently, `go.work` could hint at workspace boundaries.

## What Changes

- **Cargo**: Before parsing member crates, scan for workspace `Cargo.toml` files and extract `[workspace.dependencies]`. Pass this context so members can resolve `workspace = true` deps to actual versions.
- **npm**: Strip `workspace:` prefix from version requirements. The underlying dep name matching already handles internal detection.
- **Go**: Parse `go.work` files to extract `use` directives. Store workspace membership as metadata on Go packages found via `go.work`.

## Capabilities

### Modified Capabilities
- `manifest-parsing`: Cargo parser resolves workspace-inherited deps; npm parser normalizes workspace protocol versions
- `package-discovery`: `go.work` added as a discoverable manifest type

### New Capabilities
- `workspace-context`: Pre-processing step that collects workspace-level metadata before individual manifest parsing

## Impact

- `src/index/mod.rs` — add workspace context collection pass before main parse loop
- `src/index/cargo.rs` — accept optional workspace deps map, resolve `workspace = true`
- `src/index/npm.rs` — strip `workspace:` prefix from version strings
- `src/index/go.rs` — new `go.work` parsing (or separate parser)
- `src/index/manifest.rs` — extend `ManifestParser` trait or add separate workspace context type
- No changes to DB schema, MCP server, or query layer
