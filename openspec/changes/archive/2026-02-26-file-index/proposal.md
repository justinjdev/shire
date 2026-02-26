## Why

Shire answers "what packages exist" and "where is `AuthService` defined" but not "where is the auth middleware file" or "find all .proto files." AI agents currently fall back to grep/find to locate files by name or extension, which is slow in large repos and requires shell access. A file-level index lets the MCP server answer file location questions instantly without filesystem access.

## What Changes

- **File walking** during `shire build`: walk ALL files in the repo (not just manifests), respecting the same exclude directories, and store their paths in the database
- **New `files` table** in SQLite storing relative path, owning package (nullable), file extension, and size in bytes
- **FTS5 index on file paths** for fast path/name search
- **Package association**: each file is associated with the package whose directory is its nearest ancestor
- **2 new MCP tools**: `search_files` (FTS on paths, with optional package/extension filter) and `list_package_files` (all files belonging to a package, with optional extension filter)

## Capabilities

### New Capabilities
- `file-index`: Walk all repository files, store paths with metadata, associate files with their owning package, and provide FTS search over file paths

### Modified Capabilities
- `mcp-server`: 2 new tools added (search_files, list_package_files)

## Impact

- `src/db/mod.rs` — new `files` table, `files_fts` virtual table, triggers
- `src/db/queries.rs` — new query functions for file search and listing
- `src/index/mod.rs` — new file walking phase in build pipeline, package association logic, upsert_files function
- `src/mcp/tools.rs` — 2 new tool handlers
- DB schema version bump (additive — new tables only, no migration needed)
