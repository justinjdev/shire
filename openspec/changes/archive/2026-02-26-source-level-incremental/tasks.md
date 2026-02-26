# Source-Level Incremental Indexing — Tasks

## 1. Schema

- [x] 1.1 Add `source_hashes` table to `create_schema` in `src/db/mod.rs`: `CREATE TABLE IF NOT EXISTS source_hashes (package TEXT PRIMARY KEY, content_hash TEXT NOT NULL)`
- [x] 1.2 Add `source_hashes` to the existing `test_schema_creates_tables` test assertion

## 2. Source hash computation

- [x] 2.1 Add `compute_source_hash(repo_root, package_path, package_kind) -> Result<String>` to `src/index/hash.rs`
  - Walk source files with `walker::walk_source_files()` (already returns sorted paths)
  - Hash each file with `hash_file()`
  - Concatenate hex hashes, SHA-256 the concatenation
  - Return empty-content hash if no source files found
- [x] 2.2 Unit test: known directory with 2 source files produces deterministic hash
- [x] 2.3 Unit test: adding a file changes the hash
- [x] 2.4 Unit test: empty directory produces consistent sentinel hash

## 3. Store source hash after symbol extraction (Phase 7)

- [x] 3.1 After symbol extraction in Phase 7, compute source hash for each `parsed_package` and upsert into `source_hashes`
- [x] 3.2 Add helper: `upsert_source_hash(conn, package, hash)` — `INSERT OR REPLACE INTO source_hashes (package, content_hash) VALUES (?1, ?2)`

## 4. Source-level incremental re-extraction (Phase 8)

- [x] 4.1 After Phase 7, iterate over `diff.unchanged` manifests
- [x] 4.2 For each unchanged manifest, look up the package name, path, and kind from the `packages` table (query by `path = relative_dir`)
- [x] 4.3 Compute current source hash via `compute_source_hash()`
- [x] 4.4 Load stored source hash from `source_hashes` table
- [x] 4.5 If source hash differs or no stored hash exists, re-extract symbols and update source hash
- [x] 4.6 Track count of source-level re-extractions for summary output

## 5. Cleanup

- [x] 5.1 Clear `source_hashes` on `--force` (add `DELETE FROM source_hashes` alongside existing manifest_hashes and symbols clears)
- [x] 5.2 Delete source hash on package deletion in Phase 4 (add `DELETE FROM source_hashes WHERE package IN (SELECT name FROM packages WHERE path = ?1)` before package row deletion)

## 6. Build summary

- [x] 6.1 Update incremental build summary to include source-level re-extraction count when non-zero

## 7. Tests

- [x] 7.1 Integration test: build, then modify a source file without touching the manifest — verify symbols are updated on next build
- [x] 7.2 Integration test: build twice with no changes — verify no symbol re-extraction (source hash match)
- [x] 7.3 Integration test: `--force` clears source_hashes and recomputes them
- [x] 7.4 Integration test: delete a package — verify source_hash row is removed
- [x] 7.5 Integration test: add a new source file to an existing package — verify symbols include the new file's exports
