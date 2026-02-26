## 1. DB schema

- [x] 1.1 Add `files` table to `create_schema` in `src/db/mod.rs`: columns `id` (autoincrement PK), `path` (TEXT NOT NULL UNIQUE), `package` (TEXT nullable FK to packages.name), `extension` (TEXT NOT NULL DEFAULT ''), `size_bytes` (INTEGER NOT NULL DEFAULT 0)
- [x] 1.2 Add indexes: `idx_files_package` on `files(package)`, `idx_files_extension` on `files(extension)`
- [x] 1.3 Add `files_fts` FTS5 virtual table on `path`, content-synced with `files`
- [x] 1.4 Add FTS triggers: `files_ai` (after insert), `files_ad` (after delete), `files_au` (after update) — same pattern as packages_fts and symbols_fts
- [x] 1.5 Add `files` to `test_schema_creates_tables` test

## 2. Query functions

- [x] 2.1 Add `FileRow` struct to `src/db/queries.rs`: path, package (Option), extension, size_bytes
- [x] 2.2 Add `search_files(conn, query, package_filter, extension_filter) -> Vec<FileRow>` with FTS5 search, sanitized query, LIMIT 50
- [x] 2.3 Add `list_package_files(conn, package, extension_filter) -> Vec<FileRow>` ordered by path
- [x] 2.4 Add query tests: search by filename, search by path segment, filter by package, filter by extension, combined filters, empty query returns empty, list_package_files basic, list_package_files with extension filter

## 3. File walking

- [x] 3.1 Add `WalkedFile` struct to `src/index/mod.rs`: relative_path, extension, size_bytes
- [x] 3.2 Add `walk_files(repo_root, config) -> Result<Vec<WalkedFile>>` using WalkBuilder with same exclude config as `walk_manifests`, collecting all files
- [x] 3.3 Extract extension from filename (last extension only, lowercase, empty string if none)
- [x] 3.4 Compute file size via `entry.metadata().len()`

## 4. Package association

- [x] 4.1 Add `associate_files_with_packages()` function: given list of file paths and list of (package_name, package_path) pairs, return file paths with associated package names
- [x] 4.2 Sort package paths by length descending so longest prefix matches first
- [x] 4.3 For each file, check if its directory starts with a package path; assign to the first (longest) match
- [x] 4.4 Handle root-level packages (path = `""`) as catch-all
- [x] 4.5 Handle files outside any package: package = NULL

## 5. DB operations

- [x] 5.1 Add `upsert_files(conn, files)` function: clear all rows from `files`, bulk insert new rows with prepared statement
- [x] 5.2 Add `file_count` to `shire_meta` after file indexing

## 6. Build pipeline integration

- [x] 6.1 Add phase 9 to `build_index`: after source-level incremental, call `walk_files`, associate with packages, call `upsert_files`
- [x] 6.2 Collect all known package (name, path) pairs for association — include both newly parsed and previously existing packages
- [x] 6.3 Add file count to build summary output (both full and incremental formats)
- [x] 6.4 `--force` behavior: files are always fully rebuilt, so no special handling needed beyond existing clear

## 7. MCP tools

- [x] 7.1 Add `SearchFilesParams` struct: query (String), package (Option<String>), extension (Option<String>)
- [x] 7.2 Add `ListPackageFilesParams` struct: package (String), extension (Option<String>)
- [x] 7.3 Implement `search_files` tool handler: validate non-empty query, call queries::search_files, return JSON
- [x] 7.4 Implement `list_package_files` tool handler: call queries::list_package_files, return JSON

## 8. Tests

- [x] 8.1 Unit test: `files` table exists after schema creation
- [x] 8.2 Unit test: `search_files` FTS query matches by filename
- [x] 8.3 Unit test: `search_files` FTS query matches by path segment
- [x] 8.4 Unit test: `search_files` with package filter
- [x] 8.5 Unit test: `search_files` with extension filter
- [x] 8.6 Unit test: `list_package_files` returns all files for a package ordered by path
- [x] 8.7 Unit test: `list_package_files` with extension filter
- [x] 8.8 Integration test: `build_index` on fixture repo populates files table with correct paths and package associations
- [x] 8.9 Integration test: files in excluded directories are not indexed
- [x] 8.10 Integration test: files outside any package have NULL package
- [x] 8.11 Integration test: file count appears in shire_meta
- [x] 8.12 Integration test: rebuild clears and repopulates files (add a file, rebuild, verify it appears)
