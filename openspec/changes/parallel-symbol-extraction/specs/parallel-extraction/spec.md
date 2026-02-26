# Parallel Symbol Extraction

## ADDED Requirements

### Requirement: Parallel symbol extraction for new/changed packages

Phase 7 of `build_index` SHALL execute symbol extraction and source hash computation in parallel across packages using `rayon::par_iter`. Database inserts MUST remain sequential on the main thread.

#### Scenario: Phase 7 parallel extraction

- **WHEN** `build_index` reaches Phase 7 with one or more new/changed packages in `parsed_packages`
- **THEN** `extract_symbols_for_package` and `compute_source_hash` SHALL be called in parallel across packages
- **AND** all extraction results MUST be collected before any database writes begin
- **AND** `upsert_symbols` and `upsert_source_hash` MUST be called sequentially for each package's results

#### Scenario: Phase 7 extraction failure isolation

- **WHEN** symbol extraction fails for one package during parallel Phase 7 execution
- **THEN** the failure MUST NOT prevent extraction from completing for other packages
- **AND** the failure MUST be reported via stderr warning, consistent with current behavior

### Requirement: Parallel source-level incremental re-extraction

Phase 8 of `build_index` SHALL execute source hash computation and conditional symbol re-extraction in parallel across unchanged packages. Database reads for stored hashes MUST be performed before the parallel section. Database inserts MUST remain sequential.

#### Scenario: Phase 8 parallel hash computation

- **WHEN** `build_index` reaches Phase 8 with unchanged manifests in `diff.unchanged`
- **THEN** package info and stored source hashes SHALL be queried from the database before entering parallel execution
- **AND** `compute_source_hash` SHALL run in parallel across unchanged packages
- **AND** packages whose current hash differs from the stored hash SHALL have `extract_symbols_for_package` called in parallel

#### Scenario: Phase 8 conditional re-extraction

- **WHEN** parallel source hash computation completes for unchanged packages
- **THEN** only packages with changed source hashes SHALL have symbols re-extracted
- **AND** packages with matching source hashes MUST NOT have symbols re-extracted
- **AND** `upsert_symbols` and `upsert_source_hash` MUST be called sequentially for re-extracted packages

#### Scenario: Phase 8 re-extraction count

- **WHEN** Phase 8 parallel execution completes
- **THEN** the `num_source_reextracted` count MUST equal the number of packages that had changed source hashes and were successfully re-extracted
- **AND** this count MUST be identical to what sequential execution would produce

### Requirement: Deterministic database output

Parallel execution MUST produce database state identical to sequential execution. The final content of `symbols`, `source_hashes`, and `shire_meta` tables MUST NOT depend on execution order or thread scheduling.

#### Scenario: Symbol table equivalence

- **WHEN** `build_index` completes with parallel extraction enabled
- **THEN** the `symbols` table MUST contain exactly the same rows as sequential execution would produce for the same inputs
- **AND** the `source_hashes` table MUST contain exactly the same rows as sequential execution would produce

#### Scenario: Build summary correctness

- **WHEN** `build_index` completes with parallel extraction
- **THEN** the `symbol_count` in `shire_meta` MUST reflect the actual number of rows in the `symbols` table
- **AND** the `package_count` in `shire_meta` MUST reflect the actual number of rows in the `packages` table
- **AND** the build summary printed to stdout MUST report correct counts

### Requirement: Thread safety of extraction functions

`extract_symbols_for_package` and `compute_source_hash` MUST be safe to call concurrently from multiple rayon worker threads without data races or shared mutable state.

#### Scenario: No shared mutable state in symbol extraction

- **WHEN** `extract_symbols_for_package` is called concurrently for different packages
- **THEN** each invocation MUST create its own tree-sitter parser instance
- **AND** each invocation MUST operate only on its own package's source files
- **AND** no mutable state SHALL be shared between concurrent invocations

#### Scenario: No shared mutable state in hash computation

- **WHEN** `compute_source_hash` is called concurrently for different packages
- **THEN** each invocation MUST create its own SHA-256 hasher instance
- **AND** each invocation MUST operate only on its own package's source files
- **AND** no mutable state SHALL be shared between concurrent invocations
