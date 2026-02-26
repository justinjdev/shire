# Batch Writes

## ADDED Requirements

### Requirement: Transaction-wrapped build phases

Each build phase that performs database writes SHALL be wrapped in an explicit transaction. The transaction SHALL be committed on success and rolled back on error. Previously committed phases SHALL remain intact regardless of later phase failures.

#### Scenario: Successful build commits all phase transactions

- **WHEN** `shire build` completes without errors
- **THEN** each write phase (parse, remove-deleted, update-hashes, extract-symbols, source-re-extraction, index-files) has its writes committed in a single transaction per phase
- **AND** the database content is identical to a build without explicit transactions

#### Scenario: Error during a phase rolls back that phase only

- **WHEN** an error occurs during a write phase (e.g., symbol extraction fails mid-phase)
- **THEN** the transaction for that phase SHALL be rolled back
- **AND** all writes from previously committed phases SHALL remain in the database
- **AND** the error SHALL be propagated to the caller

#### Scenario: Force build uses transactions

- **WHEN** `shire build --force` is invoked
- **THEN** the initial DELETE statements (manifest_hashes, symbols, source_hashes) SHALL be wrapped in a transaction
- **AND** all subsequent write phases SHALL each be wrapped in their own transactions

### Requirement: Batch symbol inserts

Symbol inserts SHALL be batched into multi-row INSERT statements with up to 100 symbols per statement. The final database state MUST be identical to inserting symbols individually.

#### Scenario: Package with fewer than 100 symbols

- **WHEN** symbols are inserted for a package with N symbols where N <= 100
- **THEN** all N symbols SHALL be inserted in a single multi-row INSERT statement
- **AND** the `symbols` table SHALL contain exactly N rows for that package

#### Scenario: Package with more than 100 symbols

- **WHEN** symbols are inserted for a package with N symbols where N > 100
- **THEN** symbols SHALL be inserted in ceil(N / 100) multi-row INSERT statements
- **AND** the last statement MAY contain fewer than 100 rows
- **AND** the `symbols` table SHALL contain exactly N rows for that package

#### Scenario: Symbol data integrity after batching

- **WHEN** symbols are batch-inserted for a package
- **THEN** every symbol's package, name, kind, signature, file_path, line, visibility, parent_symbol, return_type, and parameters values MUST match the input data exactly
- **AND** the FTS index (`symbols_fts`) SHALL be updated via existing triggers for each inserted row

#### Scenario: Delete-before-insert preserved

- **WHEN** `upsert_symbols` is called for a package
- **THEN** all existing symbols for that package SHALL be deleted before the batch insert
- **AND** this delete + batch insert sequence SHALL occur within the same transaction

### Requirement: Batch file inserts

File inserts SHALL be batched into multi-row INSERT statements with up to 500 files per statement. The final database state MUST be identical to inserting files individually.

#### Scenario: Fewer than 500 files

- **WHEN** files are inserted and the total file count N <= 500
- **THEN** all N files SHALL be inserted in a single multi-row INSERT statement

#### Scenario: More than 500 files

- **WHEN** files are inserted and the total file count N > 500
- **THEN** files SHALL be inserted in ceil(N / 500) multi-row INSERT statements
- **AND** the last statement MAY contain fewer than 500 rows

#### Scenario: File data integrity after batching

- **WHEN** files are batch-inserted
- **THEN** every file's path, package, extension, and size_bytes values MUST match the input data exactly
- **AND** the FTS index (`files_fts`) SHALL be updated via existing triggers for each inserted row

#### Scenario: Delete-all-before-insert preserved

- **WHEN** `upsert_files` is called
- **THEN** all existing rows in `files` SHALL be deleted before the batch insert
- **AND** this delete + batch insert sequence SHALL occur within the same transaction

### Requirement: Batch hash upserts

Manifest hash and source hash upserts SHALL be batched within their respective transactions. Individual per-row upserts SHALL be replaced with multi-row INSERT OR REPLACE statements.

#### Scenario: Manifest hash batch upsert

- **WHEN** manifest hashes are updated in Phase 6
- **THEN** all manifest hash upserts for the phase SHALL be executed as multi-row INSERT OR REPLACE statements within a single transaction
- **AND** the `manifest_hashes` table SHALL contain the correct path and content_hash for every parsed manifest

#### Scenario: Source hash upserts within symbol extraction

- **WHEN** source hashes are stored during Phase 7 (symbol extraction for changed packages) or Phase 8 (source-level re-extraction)
- **THEN** source hash upserts SHALL occur within the same transaction as the symbol writes for that phase
- **AND** the `source_hashes` table SHALL contain the correct package and content_hash

### Requirement: Error rollback

WHEN an error occurs during a transaction THEN the transaction SHALL be rolled back and the error propagated. Previously committed phases SHALL remain intact.

#### Scenario: Rollback on symbol insertion failure

- **WHEN** a symbol batch INSERT fails (e.g., constraint violation, I/O error)
- **THEN** the entire transaction for that phase SHALL be rolled back
- **AND** the `symbols` table SHALL not contain partial results from that phase
- **AND** the error SHALL be returned to the caller

#### Scenario: Rollback on file insertion failure

- **WHEN** a file batch INSERT fails
- **THEN** the entire transaction for that phase SHALL be rolled back
- **AND** the `files` table SHALL retain its state from before the failed phase began
- **AND** the error SHALL be returned to the caller

#### Scenario: Prior phases unaffected by later failure

- **WHEN** Phase 7 (symbol extraction) fails after Phase 3 (parse) and Phase 6 (hash update) have committed
- **THEN** packages inserted in Phase 3 SHALL remain in the database
- **AND** manifest hashes stored in Phase 6 SHALL remain in the database
- **AND** only Phase 7's writes SHALL be rolled back

### Requirement: Behavioral equivalence

The database content after a build with batch operations MUST be identical to a build without batch operations, given the same inputs. Batching is a performance optimization that SHALL NOT alter observable behavior.

#### Scenario: Full build equivalence

- **WHEN** `shire build --force` runs on a repository with batch operations enabled
- **THEN** the `packages`, `dependencies`, `symbols`, `files`, `manifest_hashes`, `source_hashes`, and `shire_meta` tables SHALL contain the same rows and values as a build without batching

#### Scenario: Incremental build equivalence

- **WHEN** `shire build` runs incrementally (some packages changed, some unchanged) with batch operations enabled
- **THEN** the same packages SHALL be parsed, the same symbols re-extracted, the same files inserted, and the same hashes stored as without batching

#### Scenario: FTS indexes consistent

- **WHEN** a build completes with batch operations
- **THEN** the FTS indexes (`packages_fts`, `symbols_fts`, `files_fts`) SHALL return the same search results as a build without batching
- **AND** FTS triggers fire identically for batch-inserted and individually-inserted rows
