## Why

DB writes throughout the build pipeline happen as individual statements inside loops — one `INSERT OR REPLACE` per package, per symbol, per file, per hash. SQLite performance degrades significantly with many small writes outside explicit transactions. Wrapping phases in transactions and batching inserts would reduce write overhead, especially on full builds with thousands of symbols and files.

## What Changes

- Wrap each build phase in an explicit `BEGIN`/`COMMIT` transaction
- Batch symbol inserts using multi-row `INSERT` statements instead of per-row prepared statements
- Batch file inserts similarly
- Batch hash upserts for manifest and source hashes
- Ensure error handling rolls back on failure within each transaction

## Capabilities

### New Capabilities
- `batch-writes`: Transaction-wrapped batch DB operations for build phases

### Modified Capabilities

## Impact

- `src/index/mod.rs` — `build_index` wraps phase groups in transactions
- `src/db/queries.rs` — New batch insert functions for symbols, files, hashes
- No schema changes
- No behavioral changes — same data, same ordering, just faster writes
- Biggest win on full builds and large repos with many symbols/files
