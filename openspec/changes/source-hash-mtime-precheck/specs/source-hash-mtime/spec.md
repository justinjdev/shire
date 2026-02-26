# Source Hash Mtime Pre-check Spec

## ADDED Requirements

### Requirement: Mtime pre-check function

The system SHALL provide a function that checks whether any source file in a package directory has been modified since a given timestamp, using filesystem mtime. This function enables skipping expensive source hash computation for packages whose files have not been touched.

#### Scenario: No files newer than timestamp

- **WHEN** `has_newer_source_files` is called with a package directory, extension filters, and a reference timestamp
- **AND** all source files in the directory have mtime less than or equal to the reference timestamp
- **THEN** the function SHALL return false

#### Scenario: At least one file newer than timestamp

- **WHEN** `has_newer_source_files` is called with a package directory, extension filters, and a reference timestamp
- **AND** at least one source file has mtime strictly greater than the reference timestamp
- **THEN** the function SHALL return true
- **AND** the function MUST short-circuit on the first newer file found (no further files are checked)

#### Scenario: Same walker and extension filters as symbol extraction

- **WHEN** `has_newer_source_files` walks the package directory
- **THEN** it MUST use the same walker and extension filters as `compute_source_hash` and symbol extraction
- **AND** only files matching the package kind's extensions are checked

#### Scenario: Mtime unavailable or error

- **WHEN** mtime cannot be read for any source file (permission error, unsupported filesystem, or other I/O error)
- **THEN** the function SHALL return true (conservatively assume files may have changed)

### Requirement: Mtime-based skip in Phase 8

Phase 8 SHALL use the mtime pre-check to skip source hash computation for unchanged packages when possible.

#### Scenario: Stored hashed_at exists and no files newer

- **WHEN** Phase 8 checks an unchanged package
- **AND** the stored `hashed_at` timestamp exists for that package in `source_hashes`
- **AND** `has_newer_source_files` returns false for the package directory using `hashed_at` as the reference timestamp
- **THEN** source hash computation SHALL be skipped entirely for that package
- **AND** symbol re-extraction SHALL NOT occur

#### Scenario: Stored hashed_at exists but files are newer

- **WHEN** Phase 8 checks an unchanged package
- **AND** the stored `hashed_at` timestamp exists for that package in `source_hashes`
- **AND** `has_newer_source_files` returns true for the package directory
- **THEN** the system SHALL fall through to full source hash computation
- **AND** the computed hash SHALL be compared against the stored hash as before

#### Scenario: No stored hashed_at

- **WHEN** Phase 8 checks an unchanged package
- **AND** the `hashed_at` column is NULL for that package (e.g., first build after migration)
- **THEN** the system SHALL fall through to full source hash computation
- **AND** the `hashed_at` timestamp SHALL be stored alongside the computed hash

### Requirement: Mtime fallback to full hash

The mtime pre-check is a heuristic optimization. The system MUST always preserve correctness by falling through to full hash computation when the mtime check is inconclusive.

#### Scenario: Mtime check returns true but content unchanged

- **WHEN** `has_newer_source_files` returns true (e.g., file was touched but content is identical)
- **AND** the full source hash matches the stored hash
- **THEN** symbols SHALL NOT be re-extracted
- **AND** the `hashed_at` timestamp SHALL be updated to reflect the new hash computation time
