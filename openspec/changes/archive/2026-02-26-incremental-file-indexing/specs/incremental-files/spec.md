# Incremental Files

## ADDED Requirements

### Requirement: File-tree hash computation

The system SHALL compute an aggregate hash of the file tree by walking all files, collecting (path, size_bytes) tuples, sorting lexicographically by path, and hashing the concatenation. The hash SHALL be stored in `shire_meta` with key `file_tree_hash`.

#### Scenario: Hash computed from file metadata

- **WHEN** `shire build` runs against a repository
- **THEN** all files are walked (respecting the same exclude list as file discovery)
- **AND** each file's relative path and size in bytes are collected as a tuple
- **AND** the tuples are sorted lexicographically by path
- **AND** a SHA-256 hash is computed over the sorted concatenation
- **AND** the resulting hash is stored in `shire_meta` with key `file_tree_hash`

#### Scenario: Hash changes on file addition

- **WHEN** a new file is added to the repository
- **THEN** the recomputed file-tree hash differs from the previously stored hash

#### Scenario: Hash changes on file deletion

- **WHEN** a file is removed from the repository
- **THEN** the recomputed file-tree hash differs from the previously stored hash

#### Scenario: Hash changes on file size change

- **WHEN** a file's size in bytes changes (content modified)
- **THEN** the recomputed file-tree hash differs from the previously stored hash

#### Scenario: Hash unchanged when no structural changes

- **WHEN** no files are added, removed, or resized since the last build
- **THEN** the recomputed file-tree hash matches the stored hash

### Requirement: File index skip on unchanged tree

WHEN the file-tree hash matches the stored value, Phase 9 SHALL be skipped entirely and the `files` table SHALL remain unchanged.

#### Scenario: Skip file indexing

- **WHEN** `shire build` runs
- **AND** the computed file-tree hash matches the `file_tree_hash` value in `shire_meta`
- **THEN** the `files` table is NOT cleared
- **AND** no rows are inserted, updated, or deleted in the `files` table
- **AND** Phase 9 completes without modifying the database

#### Scenario: File count preserved on skip

- **WHEN** Phase 9 is skipped due to unchanged file-tree hash
- **THEN** the `file_count` value in `shire_meta` SHALL be read from the existing `files` table (via `SELECT COUNT(*)`)
- **AND** the build summary reports the correct file count

### Requirement: File index rebuild on changed tree

WHEN the file-tree hash differs from the stored value, the `files` table SHALL be fully rebuilt using the current behavior.

#### Scenario: Full rebuild on changed tree

- **WHEN** `shire build` runs
- **AND** the computed file-tree hash does NOT match the `file_tree_hash` value in `shire_meta`
- **THEN** all rows in the `files` table are deleted
- **AND** all walked files are re-inserted with current path, package, extension, and size_bytes
- **AND** the new file-tree hash is stored in `shire_meta` with key `file_tree_hash`

#### Scenario: Full rebuild on first build

- **WHEN** `shire build` runs
- **AND** no `file_tree_hash` key exists in `shire_meta`
- **THEN** the `files` table is fully rebuilt
- **AND** the computed file-tree hash is stored in `shire_meta`

### Requirement: File-tree hash on force build

WHEN the `--force` flag is used, the file-tree hash SHALL be cleared and recomputed after full file indexing.

#### Scenario: Force clears and recomputes hash

- **WHEN** `shire build --force` runs
- **THEN** the `file_tree_hash` key is deleted from `shire_meta` before Phase 9
- **AND** the `files` table is fully rebuilt (same as current behavior)
- **AND** a new file-tree hash is computed and stored in `shire_meta`
