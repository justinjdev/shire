# Package Discovery

## MODIFIED Requirements

### Requirement: Default manifests

#### Scenario: go.work included in defaults

- **WHEN** `[discovery].manifests` is not specified
- **THEN** defaults include `go.work` alongside existing manifest types

### Requirement: go.work discovery

#### Scenario: go.work parsed for workspace context

- **WHEN** a `go.work` file is discovered during walk
- **THEN** its `use` directives are extracted to identify workspace member directories
- **AND** the `go.work` file itself is NOT indexed as a package

#### Scenario: Go workspace member metadata

- **WHEN** a `go.mod` package exists in a directory listed in a `go.work` `use` directive
- **THEN** the package's metadata includes `{"go_workspace": true}`
