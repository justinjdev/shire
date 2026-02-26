# Incremental Indexing Tasks

## 1. Schema

- [x] 1.1 Add `manifest_hashes` table to `create_schema` in `src/db/mod.rs`
- [x] 1.2 Add test for new table existence in `test_schema_creates_tables`

## 2. Content hashing utility

- [x] 2.1 Add `sha2` crate to `Cargo.toml`
- [x] 2.2 Create `src/index/hash.rs` with `fn hash_file(path: &Path) -> Result<String>` (hex-encoded SHA-256)
- [x] 2.3 Unit test: hash of known content produces expected digest

## 3. Refactor build_index into phases

- [x] 3.1 Extract walk phase: returns `Vec<(PathBuf, String, String)>` of (manifest_path, relative_dir, content_hash)
- [x] 3.2 Add DB query helpers: `load_stored_hashes(conn) -> HashMap<String, String>` and `save_hashes / delete_hashes`
- [x] 3.3 Add diff logic: given walked manifests + stored hashes, produce `new`, `changed`, `removed`, `unchanged` sets
- [x] 3.4 Wire it together: parse only new + changed, delete removed, recompute is_internal
- [x] 3.5 Keep existing full-rebuild path for first build (empty manifest_hashes) and --force

## 4. is_internal recomputation

- [x] 4.1 Replace per-dep is_internal check with post-insert SQL UPDATE across all deps
- [x] 4.2 SQL must handle both package name matches and Go module path aliases

## 5. CLI --force flag

- [x] 5.1 Add `--force` flag to `Build` command in `src/main.rs`
- [x] 5.2 Pass force flag to `build_index`, clear `manifest_hashes` when set

## 6. Summary output

- [x] 6.1 Print incremental summary: added/updated/removed/skipped counts
- [x] 6.2 Full build prints current-style summary

## 7. Tests

- [x] 7.1 Unit test: second build with no changes parses nothing (skipped = total)
- [x] 7.2 Unit test: modifying one manifest re-parses only that one
- [x] 7.3 Unit test: deleting a manifest removes its package + deps
- [x] 7.4 Unit test: adding a manifest picks it up as new
- [x] 7.5 Unit test: is_internal updates when package set changes
- [x] 7.6 Unit test: --force triggers full rebuild even with valid hashes
- [x] 7.7 Integration test unchanged (output format matches for first build)
