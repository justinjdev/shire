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
- **THEN** the following directories are excluded: `node_modules`, `vendor`, `dist`, `.build`, `target`, `third_party`, `.shire`, `.gradle`, `build`

### Requirement: Manifest filename filtering

Only files whose names exactly match a known manifest filename are considered for parsing.

#### Scenario: Recognized manifest filenames

- **WHEN** a file is named `package.json`, `go.mod`, `go.work`, `Cargo.toml`, `pyproject.toml`, `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, or `settings.gradle.kts`
- **AND** that filename is in the enabled manifests list
- **THEN** the file is passed to the corresponding parser

#### Scenario: Unrecognized files are ignored

- **WHEN** a file does not match any known manifest filename
- **THEN** it is silently skipped

### Requirement: Parse failure resilience

Individual manifest parse failures do not abort the build.

### Requirement: go.work discovery

#### Scenario: go.work parsed for workspace context

- **WHEN** a `go.work` file is discovered during walk
- **THEN** its `use` directives are extracted to identify workspace member directories
- **AND** the `go.work` file itself is NOT indexed as a package

#### Scenario: Go workspace member metadata

- **WHEN** a `go.mod` package exists in a directory listed in a `go.work` `use` directive
- **THEN** the package's metadata includes `{"go_workspace": true}`

### Requirement: Parse failure resilience

Individual manifest parse failures do not abort the build.

#### Scenario: One manifest fails to parse

- **WHEN** a manifest file is discovered but fails to parse
- **THEN** the failure is recorded with file path and error message
- **AND** indexing continues with remaining manifests
- **AND** the failure count and details are printed to stderr after indexing

### Requirement: Source file scanning

After packages are indexed, source files within each package directory are scanned for symbol extraction.

#### Scenario: Source files discovered per ecosystem

- **WHEN** symbol extraction runs for a package
- **THEN** the package's directory is walked for source files matching the package's ecosystem
- **AND** file extensions are: `.ts`, `.tsx`, `.js`, `.jsx` (npm), `.go` (go), `.rs` (cargo), `.py` (python), `.java`, `.kt` (maven, gradle)

#### Scenario: Excluded directories

- **WHEN** walking a package directory for source files
- **THEN** directories named `node_modules`, `target`, `dist`, `.build`, `vendor`, `test`, `tests`, `__tests__`, `__pycache__` are skipped

### Requirement: settings.gradle discovery

#### Scenario: settings.gradle parsed for workspace context

- **WHEN** a `settings.gradle` or `settings.gradle.kts` file is discovered during walk
- **THEN** its `include` directives are extracted to identify multi-project member directories
- **AND** the `settings.gradle` file itself is NOT indexed as a package

#### Scenario: Gradle workspace member metadata

- **WHEN** a `build.gradle` or `build.gradle.kts` package exists in a directory listed in a `settings.gradle` `include` directive
- **THEN** the package's metadata includes `{"gradle_workspace": true}`

### Requirement: Maven parent POM context collection

#### Scenario: Parent POMs collected for child resolution

- **WHEN** `shire build` runs and discovers `pom.xml` files
- **THEN** parent POMs (those with `<modules>` and `<packaging>pom</packaging>`) are collected for workspace context
- **AND** their `groupId`, `version`, and `dependencyManagement` entries are available to child POM parsers
