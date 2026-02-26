## Why

Phase 8 computes a source hash for every unchanged package on every build by reading and SHA-256 hashing every source file. For a monorepo with hundreds of packages where most sources haven't changed, this is wasted I/O. A quick mtime check against the stored hash timestamp could skip the expensive hash computation entirely for packages whose files haven't been touched.

## What Changes

- Store the build timestamp when a source hash is computed (new column or use `shire_meta.indexed_at`)
- Before computing a source hash, check if any source file in the package has an mtime newer than the stored hash timestamp
- If no files are newer, skip the hash computation entirely
- Fall back to full hash comparison if mtime check is inconclusive (e.g., clock skew, missing mtime)

## Capabilities

### New Capabilities
- `source-hash-mtime`: Mtime-based pre-check to skip source hash computation for untouched packages

### Modified Capabilities
- `incremental-build`: Source hash tracking gains an mtime fast-path that skips hash computation when no files have been modified since the last build

## Impact

- `src/index/hash.rs` — New `has_newer_files` function that walks source files checking mtime
- `src/index/mod.rs` — Phase 8 calls mtime check before `compute_source_hash`
- `src/db/mod.rs` — May need timestamp column on `source_hashes` table (schema migration)
- Biggest win on incremental builds where most packages are untouched
