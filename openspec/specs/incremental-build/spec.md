# Incremental Build Spec

## ADDED Requirements

### Requirement: Content hash tracking

The system stores a SHA-256 content hash for each manifest file after successful parsing. Hashes persist across builds in the `manifest_hashes` table.

#### Scenario: First build (no prior hashes)

- **WHEN** `shire build` runs and `manifest_hashes` is empty
- **THEN** all discovered manifests are parsed (equivalent to full build)
- **AND** content hashes are stored for every successfully parsed manifest

#### Scenario: Subsequent build with no changes

- **WHEN** `shire build` runs and all manifest hashes match stored values
- **THEN** no manifests are re-parsed
- **AND** the package and dependency tables are unchanged
- **AND** the summary reports 0 updated, 0 added, 0 removed

#### Scenario: One manifest changed

- **WHEN** `shire build` runs and one manifest's hash differs from stored value
- **THEN** only that manifest is re-parsed
- **AND** its package row is updated (INSERT OR REPLACE)
- **AND** its old dependencies are deleted and new ones inserted
- **AND** the stored hash is updated
- **AND** `is_internal` is recomputed for all dependencies

### Requirement: Manifest addition detection

#### Scenario: New manifest appears

- **WHEN** `shire build` runs and discovers a manifest with no stored hash
- **THEN** the new manifest is parsed and its package inserted
- **AND** a hash is stored for it

### Requirement: Manifest removal detection

#### Scenario: Manifest deleted

- **WHEN** `shire build` runs and a stored hash has no corresponding file on disk
- **THEN** the package from that manifest is deleted from `packages`
- **AND** all dependencies where `package = <deleted>` are deleted
- **AND** the stored hash row is removed

### Requirement: Force rebuild

#### Scenario: --force flag

- **WHEN** `shire build --force` is invoked
- **THEN** all hashes are cleared and a full rebuild occurs (current behavior)
- **AND** the summary reports total counts, not incremental diffs

### Requirement: is_internal recomputation

After incremental updates, `is_internal` must reflect the current full set of known packages. A package being added or removed can change whether other packages' deps are internal.

#### Scenario: New package makes existing dep internal

- **WHEN** package A depends on "foo" (external) and a new manifest adds package "foo"
- **THEN** after build, A's dependency on "foo" has `is_internal = 1`

#### Scenario: Removed package makes existing dep external

- **WHEN** package A depends on "foo" (internal) and "foo"'s manifest is deleted
- **THEN** after build, A's dependency on "foo" has `is_internal = 0`

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

#### Scenario: Source hash stored after symbol extraction

- **WHEN** symbols are extracted for a package (whether due to manifest change or source change)
- **THEN** the package's source hash is computed and stored in `source_hashes`
- **AND** subsequent builds will compare against this stored hash

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

## MODIFIED Requirements

### Requirement: Build summary output

#### Scenario: Incremental build output

- **WHEN** an incremental build completes
- **THEN** stdout prints: `Indexed N packages (A added, U updated, R removed, S skipped) into <path>`

#### Scenario: Incremental build with source-only re-extractions

- **WHEN** an incremental build completes and some packages had source-only symbol re-extraction
- **THEN** stdout includes the count of source-level re-extractions in the summary

#### Scenario: Force/first build output

- **WHEN** a full build completes (first run or --force)
- **THEN** stdout prints: `Indexed N packages into <path>` (current behavior)
