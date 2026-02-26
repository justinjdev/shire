# File Index

## Requirements

### Requirement: File discovery

All files in the repository are walked and indexed during build. The same exclude directory list used for manifest discovery is respected.

#### Scenario: Standard repo walk

- **WHEN** `shire build` runs against a repository root
- **THEN** all files in the repo are discovered recursively
- **AND** each file's relative path (from repo root), extension, and size in bytes are recorded
- **AND** hidden directories (prefixed with `.`) are traversed by default

#### Scenario: Exclude directories

- **WHEN** a directory name matches an entry in the exclude list
- **THEN** that directory and all its contents are skipped
- **AND** no files within it are indexed

#### Scenario: Default excludes

- **WHEN** no custom exclude list is configured
- **THEN** the following directories are excluded: `node_modules`, `vendor`, `dist`, `.build`, `target`, `third_party`, `.shire`

#### Scenario: Extension extraction

- **WHEN** a file has an extension (e.g., `auth.middleware.ts`)
- **THEN** the extension is stored as the final extension only (e.g., `ts`)

#### Scenario: File without extension

- **WHEN** a file has no extension (e.g., `Makefile`, `Dockerfile`)
- **THEN** the extension is stored as an empty string

### Requirement: Package association

Each file is associated with the package whose directory is its nearest ancestor.

#### Scenario: File inside a package directory

- **WHEN** a file is at `services/auth/src/middleware.ts`
- **AND** a package exists with path `services/auth`
- **THEN** the file's `package` field is set to that package's name

#### Scenario: File nested under multiple packages

- **WHEN** a file is at `services/auth/sub-pkg/lib/util.ts`
- **AND** packages exist at `services/auth` and `services/auth/sub-pkg`
- **THEN** the file is associated with the package at `services/auth/sub-pkg` (longest matching prefix)

#### Scenario: File outside any package

- **WHEN** a file is at `scripts/deploy.sh`
- **AND** no package path is a prefix of `scripts/deploy.sh`
- **THEN** the file's `package` field is NULL

#### Scenario: File at repo root

- **WHEN** a file is at the repo root (e.g., `README.md`)
- **AND** a package exists with path `` (empty string, root-level package)
- **THEN** the file is associated with that root package

### Requirement: File metadata storage

#### Scenario: Stored fields

- **WHEN** a file is indexed
- **THEN** the following fields are stored:
  - `path`: relative path from repo root (e.g., `services/auth/src/middleware.ts`)
  - `package`: name of the owning package, or NULL
  - `extension`: file extension without dot (e.g., `ts`, `go`, `rs`), or empty string
  - `size_bytes`: file size in bytes

### Requirement: FTS search on file paths

#### Scenario: Search by filename

- **WHEN** searching for `middleware`
- **THEN** files whose paths contain "middleware" are returned

#### Scenario: Search by path segment

- **WHEN** searching for `auth`
- **THEN** files whose paths contain "auth" (directory or filename) are returned

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

### Requirement: Build summary

#### Scenario: File count in output

- **WHEN** a build completes
- **THEN** the total file count is included in the build summary output
- **AND** the file count is stored in `shire_meta` as `file_count`
