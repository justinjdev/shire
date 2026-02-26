## MODIFIED Requirements

### Requirement: Manifest filename filtering

Only files whose names exactly match a known manifest filename are considered for parsing.

#### Scenario: Recognized manifest filenames

- **WHEN** a file is named `package.json`, `go.mod`, `go.work`, `Cargo.toml`, `pyproject.toml`, `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, or `settings.gradle.kts`
- **AND** that filename is in the enabled manifests list
- **THEN** the file is passed to the corresponding parser

#### Scenario: Unrecognized files are ignored

- **WHEN** a file does not match any known manifest filename
- **THEN** it is silently skipped

## MODIFIED Requirements

### Requirement: Default excludes

#### Scenario: Default excludes

- **WHEN** no custom exclude list is configured
- **THEN** the following directories are excluded: `node_modules`, `vendor`, `dist`, `.build`, `target`, `third_party`, `.shire`, `.gradle`, `build`

## ADDED Requirements

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

### Requirement: Source file scanning — JVM extensions

#### Scenario: Source files discovered for JVM packages

- **WHEN** symbol extraction runs for a `maven` or `gradle` package
- **THEN** the package's directory is walked for source files with extensions `.java` and `.kt`
