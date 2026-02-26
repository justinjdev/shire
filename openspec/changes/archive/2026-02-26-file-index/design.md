# File Index — Design

## Context

Shire indexes packages, dependencies, and symbols but has no awareness of individual files in the repo. AI agents fall back to grep/find to locate files by name, extension, or path. Adding a file-level index lets the MCP server answer "where is the auth middleware" or "list all .proto files" without filesystem access.

## Goals / Non-Goals

**Goals:**
- Index all files in the repo with relative paths, extensions, and sizes
- Associate each file with its nearest ancestor package
- FTS5 search across file paths
- 2 new MCP tools for file search and listing
- Full rebuild of file index on each `shire build` (clear + re-walk)

**Non-Goals:**
- File content indexing or search (only paths)
- Per-file content hashing for incremental file-level diffing (v2)
- File type detection beyond extension (e.g., MIME type)
- Tracking file modification timestamps
- Watching for file changes (daemon mode)

## Decisions

### Decision 1: Schema — new `files` table

```sql
CREATE TABLE IF NOT EXISTS files (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    path       TEXT NOT NULL UNIQUE,
    package    TEXT REFERENCES packages(name),
    extension  TEXT NOT NULL DEFAULT '',
    size_bytes INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_files_package ON files(package);
CREATE INDEX IF NOT EXISTS idx_files_extension ON files(extension);

CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
    path,
    content='files',
    content_rowid='rowid'
);
```

Content-synced FTS5 with triggers (same pattern as `packages_fts` and `symbols_fts`). The FTS table indexes only the `path` column — this is the only field users search against.

The `path` column has a UNIQUE constraint since each file appears exactly once. The `package` column is a nullable FK — files outside any package directory have NULL.

### Decision 2: FTS tokenization for path search

FTS5's default `unicode61` tokenizer splits on path separators (`/`, `.`, `-`, `_`), which means searching for "middleware" will match `services/auth/src/middleware.ts`. This is the desired behavior — no custom tokenizer needed.

### Decision 3: File walking — reuse WalkBuilder config

The file walk reuses the same `ignore::WalkBuilder` configuration as manifest discovery: same exclude directories, same hidden-directory behavior. The only difference is that the file walk collects ALL files (not just manifest filenames).

This means a single WalkBuilder pass could theoretically collect both manifests and files. However, to keep the change minimal and avoid disrupting the existing manifest pipeline:

- **Approach**: Add a separate `walk_files()` function that uses the same WalkBuilder configuration. It runs after manifest parsing is complete, so the full package set is known for association.
- **Rationale**: Manifest walking filters to specific filenames and computes content hashes. File walking collects everything and computes sizes. Combining them would complicate the existing hash-based incremental logic.

```rust
struct WalkedFile {
    relative_path: String,
    extension: String,
    size_bytes: u64,
}

fn walk_files(repo_root: &Path, config: &Config) -> Result<Vec<WalkedFile>>
```

### Decision 4: Package association algorithm

After walking all files and knowing the full package set, associate each file with its owning package:

1. Load all package paths from the DB (or from the in-memory parsed set)
2. Sort package paths by length descending (longest first)
3. For each file, find the first package path that is a prefix of the file's directory
4. If no match, package is NULL

```rust
fn associate_files_with_packages(
    files: &mut [(String, Option<String>)],  // (path, package_name)
    packages: &[(String, String)],           // (name, path)
)
```

The sorted-by-length approach ensures that `services/auth/sub-pkg` matches before `services/auth` for files under the sub-package.

Special case: a package at the repo root (path = `""`) matches all files. This is correct — root-level packages own all files not claimed by a more specific package.

### Decision 5: Integration with build pipeline

File indexing runs as a new phase after symbol extraction:

```
Phase 1: Walk manifests
Phase 1.5: Collect workspace context
Phase 2: Diff hashes
Phase 3: Parse manifests
Phase 4: Remove deleted packages
Phase 5: Recompute is_internal
Phase 6: Update manifest hashes
Phase 7: Extract symbols
Phase 8: Index files (NEW)  ← walk all files, associate with packages, insert
```

Phase 8 always does a full rebuild of the files table (clear + re-insert). This is simpler than incremental file tracking and fast enough — WalkBuilder is already fast, and SQLite bulk inserts are cheap.

The phase runs after all package mutations are complete, so the package set is final and association is accurate.

### Decision 6: upsert_files function

```rust
fn upsert_files(
    conn: &Connection,
    files: &[(String, Option<String>, String, u64)],  // (path, package, extension, size_bytes)
) -> Result<()>
```

Strategy: clear all files, then bulk insert. Use a prepared statement in a loop for the inserts.

```sql
DELETE FROM files;
INSERT INTO files (path, package, extension, size_bytes) VALUES (?1, ?2, ?3, ?4);
```

### Decision 7: Query functions

Two new functions in `db/queries.rs`:

```rust
pub struct FileRow {
    pub path: String,
    pub package: Option<String>,
    pub extension: String,
    pub size_bytes: i64,
}

pub fn search_files(
    conn: &Connection,
    query: &str,
    package_filter: Option<&str>,
    extension_filter: Option<&str>,
) -> Result<Vec<FileRow>>

pub fn list_package_files(
    conn: &Connection,
    package: &str,
    extension_filter: Option<&str>,
) -> Result<Vec<FileRow>>
```

`search_files` uses the same FTS5 sanitization pattern as `search_packages` — wrap query in double quotes. Filters are applied via JOIN + WHERE clauses on the content table.

`list_package_files` is a direct query on the `files` table filtered by package, ordered by path.

### Decision 8: MCP tool handlers

Two new tool handlers following the existing pattern:

- `search_files`: Deserialize `SearchFilesParams { query, package?, extension? }`, call `queries::search_files`, return JSON
- `list_package_files`: Deserialize `ListPackageFilesParams { package, extension? }`, call `queries::list_package_files`, return JSON

Both follow the same error handling and JSON serialization patterns as existing tools.

## Approach

1. Add `files` table, `files_fts` virtual table, and triggers to `src/db/mod.rs`
2. Add `FileRow` struct and query functions to `src/db/queries.rs`
3. Add `walk_files()` function to `src/index/mod.rs`
4. Add package association logic
5. Add `upsert_files()` function
6. Integrate as phase 8 in `build_index`
7. Handle `--force` (clear files — though files are always cleared anyway)
8. Add file count to build summary and `shire_meta`
9. Add MCP tool param structs and handlers to `src/mcp/tools.rs`
10. Tests: unit tests for queries, integration tests for build + search
