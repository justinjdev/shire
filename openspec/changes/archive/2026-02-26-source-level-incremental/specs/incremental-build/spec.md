# Incremental Build Spec

## MODIFIED Requirements

### Requirement: Source hash tracking

The system computes an aggregate SHA-256 hash of all source files within each package directory. Source hashes are stored in the `source_hashes` table and compared on subsequent builds to detect source file changes independently from manifest changes.

#### Scenario: Source hash computation

- **WHEN** source hash is computed for a package
- **THEN** all source files in the package directory are discovered using the same walker and extension filters as symbol extraction
- **AND** file paths are sorted lexicographically (for determinism)
- **AND** each file's content is hashed individually
- **AND** the individual hashes are concatenated in sorted-path order and hashed again to produce a single aggregate hash
- **AND** the aggregate hash is stored in `source_hashes` keyed by package name

#### Scenario: Source hash stored after symbol extraction

- **WHEN** symbols are extracted for a package (whether due to manifest change or source change)
- **THEN** the package's source hash is computed and stored in `source_hashes`
- **AND** subsequent builds will compare against this stored hash

#### Scenario: Source files changed but manifest unchanged

- **WHEN** `shire build` runs and a package's manifest hash matches (unchanged)
- **AND** the package's computed source hash differs from the stored source hash
- **THEN** symbols are re-extracted for that package
- **AND** the stored source hash is updated
- **AND** the package row and dependencies are NOT re-parsed or modified

#### Scenario: Source files unchanged

- **WHEN** `shire build` runs and a package's manifest hash matches (unchanged)
- **AND** the package's computed source hash matches the stored source hash
- **THEN** symbols are NOT re-extracted for that package

#### Scenario: New package gets source hash on first build

- **WHEN** a new package is discovered and parsed
- **AND** symbols are extracted for it
- **THEN** a source hash is computed and stored for the package
- **AND** subsequent builds can detect source-only changes

#### Scenario: No prior source hash

- **WHEN** a package exists in `packages` but has no row in `source_hashes` (e.g., first build after migration)
- **THEN** symbols are re-extracted for that package
- **AND** the source hash is stored

### Requirement: Source hash cleanup

#### Scenario: Deleted package removes source hash

- **WHEN** a package is removed (its manifest was deleted)
- **THEN** the corresponding row in `source_hashes` is also deleted

#### Scenario: --force clears source hashes

- **WHEN** `shire build --force` is invoked
- **THEN** all rows in `source_hashes` are cleared (in addition to `manifest_hashes` and `symbols`)
- **AND** source hashes are recomputed and stored after symbol extraction

### Requirement: Build summary output

#### Scenario: Incremental build with source-only re-extractions

- **WHEN** an incremental build completes and some packages had source-only symbol re-extraction
- **THEN** stdout includes the count of source-level re-extractions in the summary
- **AND** the format is: `Indexed N packages (A added, U updated, R removed, S skipped), M symbols (E re-extracted) into <path>`
