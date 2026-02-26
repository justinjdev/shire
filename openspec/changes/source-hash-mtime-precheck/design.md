# Source Hash Mtime Pre-check — Design

## Context

Phase 8 of `build_index` computes `compute_source_hash` for every unchanged package on every build. This walks all source files in the package directory, reads their contents, and SHA-256 hashes them. The aggregate hash is then compared against the stored hash to detect source-level changes.

For repos with hundreds of packages where most sources haven't changed between builds, this is wasted I/O. Every unchanged package pays the cost of reading and hashing every source file, even when nothing has been touched since the last build. A quick mtime check against a stored timestamp could skip the expensive hash computation entirely for packages whose files haven't been modified.

## Goals / Non-Goals

**Goals:**
- Skip source hash computation when no files have been modified since the last build
- Preserve correctness: if the mtime check is inconclusive, fall through to full hash computation
- Keep the implementation simple: no per-file tracking, no new tables

**Non-Goals:**
- Replacing source hashing entirely — mtime is a heuristic, not a guarantee; the full hash remains the source of truth
- Sub-file change detection (e.g., tracking which functions changed within a file)
- Handling clock skew across distributed systems — this is a local developer tool

## Decisions

### 1. Add `hashed_at` column to `source_hashes` table

```sql
ALTER TABLE source_hashes ADD COLUMN hashed_at TEXT;
```

The `hashed_at` column stores an ISO 8601 timestamp (`YYYY-MM-DDTHH:MM:SS.sssZ`) recording when the source hash was last computed. This provides per-package granularity with no schema migration complexity — SQLite `ALTER TABLE ADD COLUMN` is a metadata-only operation, and the column defaults to NULL for existing rows.

NULL `hashed_at` means no mtime pre-check is possible for that row, so the system falls through to full hash computation (same behavior as before this change).

### 2. New function `has_newer_source_files(dir, extensions, since_timestamp) -> bool`

A new function in `src/index/hash.rs` that walks source files using the same `walker::walk_source_files()` and `walker::extensions_for_kind()` as `compute_source_hash`, but instead of reading file contents, it stat()s each file and checks if any have an mtime newer than `since_timestamp`.

The function short-circuits on the first file with a newer mtime — it does not need to walk the entire directory tree if one newer file is found. This makes the common case (no changes) proportional to the number of source files (stat only), and the changed case fast to detect.

`stat()` is significantly cheaper than `read()` + SHA-256. On a typical filesystem, stat is a single inode lookup versus read which requires loading data blocks.

### 3. Skip logic in Phase 8

The Phase 8 loop for unchanged packages gains a pre-check before `compute_source_hash`:

```
for each unchanged package:
    load stored source hash + hashed_at from DB
    if hashed_at exists:
        if NOT has_newer_source_files(pkg_dir, extensions, hashed_at):
            skip entirely (no hash computation, no re-extraction)
    compute_source_hash (existing logic)
    compare to stored hash
    re-extract if different
    upsert source hash + hashed_at
```

If the mtime check returns false (no newer files), source hash computation is skipped entirely. If it returns true, the system falls through to the full hash computation as before. This is a "fast negative" — mtime can confirm "definitely not changed" but a positive (newer mtime) does not guarantee content changed (the file could have been touched, had metadata updated, or been rewritten with identical content). The full hash resolves the ambiguity.

### 4. Fallback on mtime unavailability

If `has_newer_source_files` encounters an error reading mtime for any file (e.g., permission denied, unsupported filesystem), it returns `true` (conservatively assume files may have changed). This causes the system to fall through to full hash computation, preserving correctness.

This handles edge cases like NFS mounts with unreliable mtime, Docker bind mounts, or filesystems that don't update mtime on writes.

### 5. `hashed_at` written alongside source hash

The `upsert_source_hash` function is updated to also store the current timestamp:

```rust
fn upsert_source_hash(conn: &Connection, package: &str, hash: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    conn.execute(
        "INSERT OR REPLACE INTO source_hashes (package, content_hash, hashed_at) VALUES (?1, ?2, ?3)",
        (package, hash, &now),
    )?;
    Ok(())
}
```

This ensures every source hash row has a corresponding `hashed_at` timestamp for use in subsequent builds. The timestamp uses UTC to avoid timezone ambiguity.

### Alternative considered: per-file hash tracking

An alternative design would store per-file hashes and only re-hash files whose mtime changed. This would be even faster for large packages where only one file changed. Rejected because:
- Requires a new table or significant schema change (one row per source file per package)
- More DB queries per package (load N file hashes, compare, upsert changed ones)
- The current aggregate hash approach is simple and correct
- The mtime pre-check already eliminates the dominant cost (the no-change case)

## Risks / Trade-offs

**Risk: Clock skew causing false negatives.** If the system clock is set backwards after a build, source file mtimes could appear older than `hashed_at` even though the files were modified after the hash was computed. Mitigation: this only affects the source-level incremental path, not manifest-level change detection. Worst case is stale symbols until the next manifest change or `--force` build. This is acceptable for a local developer tool.

**Risk: NFS/network filesystems with unreliable mtime.** Some network filesystems do not update mtime reliably or have coarse-grained timestamps (e.g., FAT32 has 2-second resolution). Mitigation: `has_newer_source_files` falls through to full hash on any error, and coarse timestamps may cause unnecessary re-hashing but never missed changes (the mtime would appear newer or equal, triggering a hash).

**Trade-off: Small schema addition.** Adding `hashed_at` to `source_hashes` is a minor schema change. The column is nullable and defaults to NULL for existing rows, so there is no migration burden. Existing rows without `hashed_at` simply skip the mtime pre-check and hash as before.

**Trade-off: chrono dependency.** Timestamp formatting requires either manual formatting or the `chrono` crate. If `chrono` is not already a dependency, this could be done with `std::time::SystemTime` and manual ISO 8601 formatting instead.
