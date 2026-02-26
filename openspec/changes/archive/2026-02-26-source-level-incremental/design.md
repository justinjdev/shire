# Source-Level Incremental Indexing — Design

## Context

Shire's incremental build tracks manifest file hashes to skip re-parsing unchanged packages. But symbol extraction is tied to manifest changes — if a source file changes without touching the manifest, symbols go stale. This is a correctness gap: the index silently serves outdated symbol data.

## Goals / Non-Goals

**Goals:**
- Detect source file changes independently from manifest changes
- Re-extract symbols when source files change, without re-parsing the manifest
- Keep the cost minimal for the no-change case (one hash comparison per package)
- Clean up source hashes on package deletion and `--force`

**Non-Goals:**
- Per-file granular hashing (re-extract only changed files) — not worth the complexity; tree-sitter is fast enough to re-extract a full package
- File watching / daemon mode (separate feature)
- Detecting changes in files outside the package directory (e.g., shared build configs)

## Decisions

### Decision 1: New `source_hashes` table

```sql
CREATE TABLE IF NOT EXISTS source_hashes (
    package      TEXT PRIMARY KEY,  -- package name (matches packages.name)
    content_hash TEXT NOT NULL      -- hex-encoded SHA-256 of aggregate source content
);
```

Keyed by package name rather than path, because symbol extraction is per-package and the `symbols` table references packages by name. This also makes cleanup on package deletion a simple `DELETE WHERE package = ?`.

Separate table rather than a column on `packages` to keep concerns separated — `packages` stores parsed manifest data, `source_hashes` stores build-pipeline state (same rationale as the separate `manifest_hashes` table).

### Decision 2: Aggregate hash computation

```
compute_source_hash(repo_root, package_path, package_kind) -> String:
    1. Walk source files using walker::walk_source_files() (same as symbol extraction)
    2. Files are already sorted by walk_source_files()
    3. For each file, compute hash_file() → individual SHA-256
    4. Concatenate all individual hashes (in sorted-path order) into one byte stream
    5. SHA-256 the concatenation → aggregate hash
```

This reuses the existing `walk_source_files()` and `hash_file()` functions. The double-hash approach (hash each file, then hash the concatenation) avoids reading all file contents into memory at once and is deterministic given sorted paths.

If a package directory has no source files (e.g., a package with only a manifest), the aggregate hash is the SHA-256 of an empty string. This is a stable sentinel that won't change unless source files appear.

### Decision 3: Modified build pipeline

Current phases:
```
Phase 1:   Walk manifests
Phase 1.5: Collect workspace context
Phase 2:   Diff manifest hashes
Phase 3:   Parse new/changed manifests
Phase 4:   Remove deleted packages
Phase 5:   Recompute is_internal
Phase 6:   Update manifest hashes
Phase 7:   Extract symbols for manifest-changed packages
```

New phase added after Phase 7:
```
Phase 8:   Source-level incremental symbol re-extraction
           - For each UNCHANGED package (manifest hash matched):
             a. Look up stored source hash from source_hashes table
             b. Compute current source hash
             c. If different (or no stored hash), re-extract symbols + update source hash
           - Also store source hashes for packages processed in Phase 7
```

Phase 8 iterates over `diff.unchanged` packages. For each one, it needs the package name and kind (looked up from the `packages` table) to compute the source hash and dispatch to the correct extractor.

This is separated from Phase 7 rather than merged into it because Phase 7 operates on `parsed_packages` (packages whose manifests were just parsed, with name/path/kind readily available), while Phase 8 operates on `diff.unchanged` (manifests that weren't parsed, requiring a DB lookup for package metadata).

### Decision 4: Source hash computation cost

Source hash computation requires walking the source directory and reading every file to hash it. This is the same I/O as symbol extraction itself. For the common case (nothing changed), this cost is unavoidable — we must read files to know if they changed.

Mitigations:
- `walk_source_files()` already skips excluded directories and non-matching extensions, limiting the file set
- `hash_file()` reads the file in one shot (no streaming needed for typical source files)
- The per-package overhead is proportional to source file count, not total repo size
- Packages with no source files get a constant-time empty hash

For very large packages, this may add noticeable overhead to the no-change case. This is acceptable because correctness outweighs speed, and `hash_file()` is cheaper than full tree-sitter parsing.

### Decision 5: --force behavior

When `--force` is set, clear `source_hashes` along with `manifest_hashes` and `symbols`:

```rust
if force {
    conn.execute("DELETE FROM manifest_hashes", [])?;
    conn.execute("DELETE FROM symbols", [])?;
    conn.execute("DELETE FROM source_hashes", [])?;
}
```

After the full rebuild, source hashes are stored for all packages, so subsequent incremental builds have a baseline to compare against.

### Decision 6: Package deletion cleanup

In Phase 4 (remove deleted packages), add:

```rust
conn.execute(
    "DELETE FROM source_hashes WHERE package IN (SELECT name FROM packages WHERE path = ?1)",
    [relative_dir],
)?;
```

This runs before the package row itself is deleted, so the subquery still resolves.

## Data Flow

```
shire build
    │
    ├── Phases 1-6: (unchanged)
    │
    ├── Phase 7: extract symbols for manifest-changed packages
    │   └── for each parsed_package:
    │       ├── extract symbols (existing)
    │       └── compute + store source hash (NEW)
    │
    ├── Phase 8: source-level incremental (NEW)
    │   └── for each unchanged package:
    │       ├── load package name, path, kind from DB
    │       ├── compute current source hash
    │       ├── compare to stored source hash
    │       └── if different or missing:
    │           ├── re-extract symbols
    │           └── update stored source hash
    │
    └── print summary (includes re-extraction count)
```
