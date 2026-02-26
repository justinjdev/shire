# Incremental Build Spec

## MODIFIED Requirements

### Requirement: Source hash tracking

The system computes an aggregate SHA-256 hash of all source files within each package directory. Source hashes are stored in the `source_hashes` table alongside a `hashed_at` timestamp and compared on subsequent builds to detect source file changes independently from manifest changes. The `hashed_at` timestamp enables mtime-based pre-checks to skip hash computation for untouched packages.

#### Scenario: Source hash computation

- **WHEN** source hash is computed for a package
- **THEN** all source files in the package directory are discovered using the same walker and extension filters as symbol extraction
- **AND** file paths are sorted lexicographically (for determinism)
- **AND** each file's content is hashed individually
- **AND** the individual hashes are concatenated in sorted-path order and hashed again to produce a single aggregate hash
- **AND** the aggregate hash is stored in `source_hashes` keyed by package name
- **AND** the current UTC timestamp SHALL be stored in the `hashed_at` column as ISO 8601 format

#### Scenario: Source files unchanged with mtime fast-path

- **WHEN** `shire build` runs and a package's manifest hash matches (unchanged)
- **AND** the package has a stored `hashed_at` timestamp in `source_hashes`
- **AND** no source files in the package directory have mtime newer than the stored `hashed_at` timestamp
- **THEN** source hash computation SHALL be skipped entirely
- **AND** symbols SHALL NOT be re-extracted for that package
- **AND** no I/O beyond stat() calls SHALL be performed for that package's source files

#### Scenario: Source files changed but manifest unchanged

- **WHEN** `shire build` runs and a package's manifest hash matches (unchanged)
- **AND** the mtime pre-check indicates newer files exist OR no `hashed_at` timestamp is stored
- **AND** the package's computed source hash differs from the stored source hash
- **THEN** symbols are re-extracted for that package
- **AND** the stored source hash is updated
- **AND** the `hashed_at` timestamp is updated
- **AND** the package row and dependencies are NOT re-parsed or modified

#### Scenario: Source files unchanged (hash path)

- **WHEN** `shire build` runs and a package's manifest hash matches (unchanged)
- **AND** the mtime pre-check indicates newer files exist OR no `hashed_at` timestamp is stored
- **AND** the package's computed source hash matches the stored source hash
- **THEN** symbols are NOT re-extracted for that package
- **AND** the `hashed_at` timestamp SHALL be updated to reflect the new computation time
