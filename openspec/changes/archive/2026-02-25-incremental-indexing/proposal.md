# Incremental Indexing

## Why

`shire build` currently does a full rebuild every time — deletes all rows, re-walks the repo, re-parses every manifest, re-inserts everything. This is O(n) in the number of packages regardless of how many actually changed.

For large monorepos (hundreds of packages), this is noticeably slow and wasteful. Most builds in practice touch 1-3 manifests. Incremental indexing should make the common case fast while keeping full rebuild available as a fallback.

## What Changes

- Track content hashes of manifest files in a new DB table
- On `shire build`, compare current file hashes to stored hashes
- Only re-parse manifests that are new or changed
- Remove packages whose manifests have been deleted
- Recompute `is_internal` flags across the full dependency set (since the set of known packages may have changed)
- Add `--force` flag to `shire build` for explicit full rebuild
- Print a summary distinguishing unchanged/updated/added/removed packages

## Impact

- `src/db/mod.rs` — new `manifest_hashes` table in schema
- `src/index/mod.rs` — split walk/parse/insert into incremental-aware phases
- `src/main.rs` — add `--force` flag to Build command
- No changes to MCP server, query layer, or parsers
