# File Index

## MODIFIED Requirements

### Requirement: Incremental behavior

#### Scenario: Rebuild on changed file tree

- **WHEN** `shire build` runs
- **AND** the computed file-tree hash differs from the stored `file_tree_hash` in `shire_meta` (or no stored hash exists)
- **THEN** all files are cleared and re-walked
- **AND** the files table is fully rebuilt
- **AND** the new file-tree hash is stored in `shire_meta`

#### Scenario: Skip on unchanged file tree

- **WHEN** `shire build` runs
- **AND** the computed file-tree hash matches the stored `file_tree_hash` in `shire_meta`
- **THEN** the files table is NOT modified
- **AND** Phase 9 is skipped entirely

#### Scenario: Force rebuild

- **WHEN** `shire build --force` is invoked
- **THEN** the stored `file_tree_hash` is cleared from `shire_meta`
- **AND** files are cleared and re-walked
- **AND** the files table is fully rebuilt
- **AND** a new file-tree hash is computed and stored
