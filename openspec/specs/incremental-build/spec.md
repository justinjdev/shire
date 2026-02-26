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

## MODIFIED Requirements

### Requirement: Build summary output

#### Scenario: Incremental build output

- **WHEN** an incremental build completes
- **THEN** stdout prints: `Indexed N packages (A added, U updated, R removed, S skipped) into <path>`

#### Scenario: Force/first build output

- **WHEN** a full build completes (first run or --force)
- **THEN** stdout prints: `Indexed N packages into <path>` (current behavior)
