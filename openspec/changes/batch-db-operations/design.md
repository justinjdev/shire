# Batch DB Operations — Design

## Context

The `build_index` pipeline in `src/index/mod.rs` performs all database writes as individual prepared statement executions inside loops. Each call to `upsert_package`, `upsert_symbols`, `upsert_source_hash`, `upsert_files`, and the manifest hash inserts runs as its own implicit SQLite transaction under autocommit mode. This means every single INSERT/REPLACE triggers a WAL sync and fsync — the most expensive part of SQLite writes. For a full build indexing hundreds of packages with thousands of symbols and files, this adds up to significant overhead from per-statement transaction commit costs.

The `upsert_symbols` function (line 204) iterates over a `Vec<SymbolInfo>` executing one INSERT per symbol with a prepared statement. `upsert_files` (line 340) does the same for files. Manifest hash stores (Phase 6, line 634) loop over `to_parse` executing one INSERT OR REPLACE per hash. Source hash stores happen individually inside Phase 7 and Phase 8 loops.

None of the build phases use explicit transactions. The connection is opened once in `db::open_or_create` with `PRAGMA journal_mode=WAL` but no further transaction management.

## Goals / Non-Goals

**Goals:**
- Wrap each major build phase that performs writes in an explicit transaction (BEGIN/COMMIT)
- Batch symbol inserts into multi-row INSERT statements to reduce per-statement overhead
- Batch file inserts into multi-row INSERT statements
- Batch hash upserts within their respective phase transactions
- Maintain identical database content after a build — same rows, same values, same ordering
- Ensure errors within a transaction trigger a rollback, leaving previously committed phases intact

**Non-Goals:**
- Schema changes — no new tables, no altered columns
- Concurrent write support or connection pooling
- Prepared statement caching (rusqlite already handles this via its statement cache)
- Changing the build phase ordering or merging phases
- Benchmarking or measuring the improvement (that is the build-timing-instrumentation change's job)

## Decisions

### 1. Explicit transactions around write phases

Wrap each build phase that performs database writes in `conn.execute_batch("BEGIN")` ... `conn.execute_batch("COMMIT")`, with a rollback on error. This groups all writes in a phase into a single WAL sync rather than one per statement.

Phases to wrap:

| Phase | Description | Current writes |
|-------|-------------|----------------|
| 3 | Parse manifests | `upsert_package` per new/changed manifest |
| 4 | Remove deleted | DELETE from source_hashes, symbols, dependencies, packages, manifest_hashes per removed manifest |
| 6 | Update manifest hashes | INSERT OR REPLACE per parsed manifest |
| 7 | Extract symbols | `upsert_symbols` + `upsert_source_hash` per parsed package |
| 8 | Source re-extraction | `upsert_symbols` + `upsert_source_hash` per stale-source package |
| 9 | Index files | `upsert_files` (DELETE all + INSERT per file) |

Phase 5 (recompute_is_internal) is a single UPDATE statement — wrapping it adds no benefit, but wrapping it for consistency is harmless.

The `--force` deletes at the top of `build_index` (lines 448-451) should also be wrapped in a single transaction since they are three related DELETEs.

The metadata writes at the end (shire_meta inserts, config overrides) are few statements and can share a single transaction.

### 2. Batch symbol inserts: up to 100 rows per multi-row INSERT

Instead of executing one `INSERT INTO symbols VALUES (...)` per symbol, construct multi-row INSERT statements:

```sql
INSERT INTO symbols (package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, parameters)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10),
       (?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20),
       ...
```

Batch size: 100 symbols per statement. Each symbol binds 10 parameters, so 100 symbols = 1000 bind variables. SQLite's compile-time limit is `SQLITE_MAX_VARIABLE_NUMBER` which defaults to 32766 — 1000 is well under this. 100 is a practical batch size that reduces statement preparation overhead by ~100x while keeping individual statement complexity reasonable.

The `upsert_symbols` function already does a `DELETE FROM symbols WHERE package = ?1` before inserting. This pattern stays the same — the batching only applies to the INSERT loop that follows.

### 3. Batch file inserts: up to 500 rows per multi-row INSERT

Same approach as symbols. Files have 4 bindable fields (path, package, extension, size_bytes), so 500 rows = 2000 bind variables. Still well under the SQLite limit.

The `upsert_files` function already does a `DELETE FROM files` before inserting. The batching applies to the INSERT loop.

500 is chosen because files have fewer columns (4 vs 10), so larger batches are practical.

### 4. Alternative considered: rusqlite's `Transaction` type

rusqlite provides a `Transaction` struct via `conn.transaction()` that wraps BEGIN/COMMIT/ROLLBACK with RAII semantics — it auto-rolls-back on drop if not committed. This is cleaner than manual `execute_batch("BEGIN")` / `execute_batch("COMMIT")`.

```rust
let tx = conn.transaction()?;
// ... writes using &tx instead of &conn ...
tx.commit()?;
```

Either approach is functionally equivalent. Prefer the `Transaction` type if the ergonomics work out — specifically, the existing helper functions (`upsert_package`, `upsert_symbols`, etc.) accept `&Connection`, and `Transaction` derefs to `Connection`, so they should work without signature changes.

### 5. Alternative considered: WAL2 or other SQLite pragma tuning

Rejected. The connection already uses WAL mode (`PRAGMA journal_mode=WAL` in `db::open_or_create`). Additional pragmas like `synchronous=NORMAL` could reduce fsync cost but trade durability. Transaction batching addresses the primary overhead (per-statement transaction commits) without weakening durability guarantees.

### 6. Hash upsert batching

Manifest hash upserts (Phase 6) and source hash upserts (Phases 7/8) follow the same pattern as symbols/files — construct multi-row INSERT OR REPLACE statements. These tables have only 2 columns (path/package + content_hash), so batches can be larger. A batch size of 500 is appropriate.

Within a transaction, the batching provides modest additional benefit (fewer prepared statement compilations), but the transaction wrapping itself provides the primary improvement.

## Risks / Trade-offs

**Risk: crash mid-transaction loses entire phase.** Under autocommit, a crash loses only the single row being written. With explicit transactions, a crash loses all writes in the current phase. **Mitigation:** Builds are fast (sub-second for most repos) and fully idempotent — re-running `shire build` reproduces the same result. Losing one phase's writes is not a data integrity concern.

**Risk: longer write transactions increase WAL file size.** Holding a write transaction open longer means the WAL file grows larger before checkpointing. **Mitigation:** Each transaction covers a single phase, not the entire build. The longest transaction (symbol extraction in Phase 7) is bounded by the number of packages with changed manifests, which is typically small on incremental builds. Full builds are the worst case but are infrequent.

**Trade-off: more complex error handling.** Each phase needs try/rollback/propagate logic instead of simple `?` propagation. The `Transaction` type (Decision 4) mitigates this with automatic rollback on drop.

**Trade-off: multi-row INSERT statements are dynamically constructed.** Unlike static prepared statements, the SQL string changes based on batch size (the last batch may be smaller). This means the SQL string must be built at runtime and cannot be cached across invocations. The cost is negligible — string formatting is cheap compared to SQLite I/O — but it is a complexity increase over the current simple prepared-statement loop.
