# Incremental Indexing Design

## Context

`build_index` currently: DELETE all → walk → parse all → insert all. This is clean but O(n) always. We want O(changed) for the common case.

## Goals / Non-Goals

**Goals:**
- Skip re-parsing unchanged manifests
- Detect added and removed manifests
- Keep `is_internal` correct after partial updates
- Maintain `--force` escape hatch for full rebuild

**Non-Goals:**
- File watching / daemon mode (separate feature)
- Lockfile-based change detection (too ecosystem-specific)
- Parallel parsing (orthogonal optimization)

## Decisions

### Decision 1: Content hashing over mtime

Use SHA-256 of file contents, not filesystem mtime.

**Rationale:** mtime is unreliable — `git checkout` resets it, CI may not preserve it, copying files changes it. Content hashing is deterministic and we already read file contents during parse. The cost of hashing is negligible compared to I/O.

### Decision 2: New `manifest_hashes` table

```sql
CREATE TABLE IF NOT EXISTS manifest_hashes (
    path         TEXT PRIMARY KEY,  -- relative manifest path (e.g., "services/auth/package.json")
    content_hash TEXT NOT NULL      -- hex-encoded SHA-256
);
```

Stored alongside packages/dependencies in the same DB. Keyed by relative manifest path (dir + filename), not package name, since workspace Cargo.toml files are now skipped and one manifest = one package.

### Decision 3: Walk-first, parse-selectively

The build splits into phases:

```
1. WALK      — discover all manifest paths, compute hashes
2. DIFF      — compare against stored hashes → changed/new/removed sets
3. PARSE     — only parse changed + new manifests
4. UPDATE DB — upsert changed/new packages, delete removed
5. RECOMPUTE — update is_internal for ALL deps (full scan)
6. CLEANUP   — update stored hashes
```

Phase 5 (recompute) is always full-scan because any add/remove changes the known-package set. This is cheap — it's a single UPDATE query joining against the packages table, no file I/O.

### Decision 4: Dependency cleanup strategy

When a manifest changes, delete all its old deps before inserting new ones:

```sql
DELETE FROM dependencies WHERE package = ?1;
-- then insert new deps
```

This is simpler than diffing old vs new deps and handles renames/removals cleanly. The `INSERT OR REPLACE` on packages handles the package row itself.

### Decision 5: is_internal recomputation via SQL

After all inserts/deletes, run a single SQL update:

```sql
UPDATE dependencies SET is_internal = (
    dependency IN (SELECT name FROM packages)
    OR dependency IN (SELECT description FROM packages WHERE kind = 'go')
);
```

This handles both direct name matches and Go module path aliases in one pass.

## Data Flow

```
shire build
    │
    ├── --force? ──yes──→ clear manifest_hashes, fall through to full build
    │
    ├── walk repo, collect (manifest_path, sha256) pairs
    │
    ├── load stored hashes from DB
    │
    ├── diff:
    │   ├── new:     in walk but not in DB
    │   ├── changed: in both, hash differs
    │   ├── removed: in DB but not in walk
    │   └── unchanged: in both, hash matches
    │
    ├── parse new + changed manifests only
    │
    ├── DB transaction:
    │   ├── delete removed packages + their deps + their hashes
    │   ├── for changed: delete old deps, upsert package, insert new deps
    │   ├── for new: insert package + deps
    │   ├── recompute is_internal (full scan)
    │   └── upsert manifest hashes for new + changed
    │
    └── print summary
```
