# Incremental File Indexing — Design

## Context

Phase 9 of `build_index` (file indexing) currently executes `DELETE FROM files` followed by a full re-walk and full re-insert on every build. The `upsert_files` function in `src/index/mod.rs` unconditionally clears the entire `files` table, walks all files via `walk_files`, associates them with packages, and re-inserts every row. For large repos with tens of thousands of files, this is unnecessary when most files don't change between builds. The package and symbol indexes already have incremental behavior via content hashing — the file index should follow the same pattern.

## Goals / Non-Goals

**Goals:**
- Skip file re-indexing entirely when no files have changed
- Detect additions, deletions, and size modifications efficiently
- Maintain the existing full-rebuild path as a fallback
- Store enough state to make the "nothing changed" check cheap

**Non-Goals:**
- Tracking file content changes — the file index only stores path, extension, size, and package association; content is never read
- Real-time file watching or filesystem event subscriptions
- Per-file or per-directory incremental updates (diffing individual changes against the DB)
- Changing what metadata is stored in the `files` table

## Decisions

### 1. Aggregate file-tree hash stored in `shire_meta`

Store a single hash in `shire_meta` with key `file_tree_hash`. Compute it by walking all files (same walk as current Phase 9), collecting `(relative_path, size_bytes)` tuples, sorting lexicographically by path, and hashing the concatenation using the same hashing approach used elsewhere in shire (SHA-256 via the existing `sha2` dependency).

**Why:** Simple, deterministic, and catches any structural change — file additions, file deletions, and file size changes all produce a different hash. No new dependencies or schema changes required beyond a new `shire_meta` key.

### 2. Skip-or-rebuild: compare hash, skip Phase 9 if unchanged

On each build: compute the file-tree hash, compare it to the stored `file_tree_hash` in `shire_meta`. If identical, skip Phase 9 entirely (no DB writes, no file re-association). If different, do a full rebuild (current `DELETE FROM files` + re-insert behavior) and store the new hash.

**Why:** The common case during iterative development is "nothing changed in the file tree" — the developer modifies file contents, not file structure. Optimizing for this case with a simple equality check gives the biggest win with minimal complexity. A full rebuild fallback is acceptable because file indexing is I/O-light (stat calls only, no content reads) and the DB operations are fast for moderate repos.

### 3. Alternative considered: per-directory hashing for partial updates

Instead of a single aggregate hash, compute hashes per directory and only re-index directories whose hash changed. This would allow partial updates — only touching the DB rows for files in changed directories.

**Rejected:** Adds significant complexity (directory-level hash tracking, partial delete/insert logic, handling of moved files across directories) for marginal benefit. The full-rebuild fallback is already fast since file indexing only does stat + insert — no content reads, no parsing. The bottleneck in large repos is symbol extraction and manifest parsing, not file indexing.

### 4. Alternative considered: mtime-based check on directories

Use directory modification times to detect changes without walking files.

**Rejected:** Directory mtime only reflects direct children, not recursive changes. A file added three levels deep doesn't update the top-level directory's mtime. We'd still need to walk the full tree to check mtimes at every level, negating the benefit. Additionally, mtime can be unreliable across filesystems (e.g., after git checkout).

### 5. `--force` always does full rebuild

When `--force` is passed, the stored `file_tree_hash` is cleared (deleted from `shire_meta`) before Phase 9 runs. This guarantees a full file table rebuild regardless of whether the tree has changed. After the rebuild, the new hash is computed and stored.

**Why:** Consistent with how `--force` works for other incremental features (manifest hashing, source hashing). The user expects `--force` to rebuild everything from scratch.

## Risks / Trade-offs

**Hash computation walks all files.** The file-tree hash requires the same filesystem walk as the current approach — `walk_files` is called either way. The savings come from skipping the DB clear and re-insert, not from skipping the walk. For repos where the walk itself is the bottleneck (millions of files), this change provides no benefit. Mitigation: the walk is stat-only and parallelized via `ignore::WalkBuilder`; the DB operations are the actual cost being avoided.

**Falls back to full rebuild on any change.** Even a single file addition triggers a complete `DELETE FROM files` + re-insert. This is suboptimal for the case where one file is added to a 50,000-file repo. Mitigation: this is the uncommon case during iterative development (file structure changes less often than file contents), and the full rebuild is fast enough that the UX impact is minimal. Per-file diffing can be added as a future optimization if profiling shows this matters.

**Hash stability across platforms.** The hash depends on path separators and file sizes. Path separators are already normalized to forward slashes by `walk_files`. File sizes from `std::fs::metadata` are consistent across platforms for the same file. No risk here.
