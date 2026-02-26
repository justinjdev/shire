# Package Discovery

## Requirements

### Requirement: Manifest file walking

The indexer walks the repository tree to discover manifest files. Only files matching enabled manifest filenames are considered.

#### Scenario: Standard monorepo walk

- **WHEN** `shire build` runs against a repository root
- **THEN** all directories are recursively traversed
- **AND** only files matching enabled manifest filenames are collected
- **AND** hidden directories (prefixed with `.`) are traversed by default

#### Scenario: Exclude directories

- **WHEN** a directory name matches an entry in the exclude list
- **THEN** that directory and all its contents are skipped entirely
- **AND** no manifests within it are discovered

#### Scenario: Default excludes

- **WHEN** no custom exclude list is configured
- **THEN** the following directories are excluded: `node_modules`, `vendor`, `dist`, `.build`, `target`, `third_party`, `.shire`

### Requirement: Manifest filename filtering

Only files whose names exactly match a known manifest filename are considered for parsing.

#### Scenario: Recognized manifest filenames

- **WHEN** a file is named `package.json`, `go.mod`, `Cargo.toml`, or `pyproject.toml`
- **AND** that filename is in the enabled manifests list
- **THEN** the file is passed to the corresponding parser

#### Scenario: Unrecognized files are ignored

- **WHEN** a file does not match any known manifest filename
- **THEN** it is silently skipped

### Requirement: Parse failure resilience

Individual manifest parse failures do not abort the build.

#### Scenario: One manifest fails to parse

- **WHEN** a manifest file is discovered but fails to parse
- **THEN** the failure is recorded with file path and error message
- **AND** indexing continues with remaining manifests
- **AND** the failure count and details are printed to stderr after indexing
