## Why

Phase 9 (file indexing) clears and rebuilds the entire `files` table on every build. For large repos with tens of thousands of files, this is unnecessary work when most files haven't changed. The file index should be incremental like the package and symbol indexes.

## What Changes

- Track an aggregate file-tree hash or per-directory mtime to detect changes
- On incremental builds, only re-index files in directories that have changed
- Remove file entries for deleted files
- Add new file entries for new files
- Update package associations when packages are added/removed
- Full rebuild still available via `--force`

## Capabilities

### New Capabilities
- `incremental-files`: Incremental file indexing that detects additions, deletions, and modifications without full table rebuild

### Modified Capabilities
- `file-index`: The "Rebuild on each build" requirement changes to incremental behavior by default, with full rebuild on `--force`

## Impact

- `src/index/mod.rs` — Phase 9 refactored from clear-and-rebuild to diff-and-update
- `src/db/mod.rs` — May need hash/mtime tracking for file index state
- `src/db/queries.rs` — New queries for selective file insert/delete
- Biggest win on repos with many files where few change between builds
