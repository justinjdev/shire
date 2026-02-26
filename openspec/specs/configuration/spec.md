# Configuration

## Requirements

### Requirement: Config file loading

#### Scenario: shire.toml present

- **WHEN** `shire.toml` exists in the repository root
- **THEN** it is parsed and its values override defaults

#### Scenario: No config file

- **WHEN** no `shire.toml` exists
- **THEN** default configuration is used

### Requirement: Discovery configuration

#### Scenario: Custom manifest list

- **WHEN** `[discovery].manifests` is specified
- **THEN** only listed manifest filenames are discovered during walk

#### Scenario: Custom exclude list

- **WHEN** `[discovery].exclude` is specified
- **THEN** listed directory names are skipped during walk

#### Scenario: Default manifests

- **WHEN** `[discovery].manifests` is not specified
- **THEN** defaults to: `package.json`, `go.mod`, `Cargo.toml`, `pyproject.toml`

### Requirement: Package overrides

#### Scenario: Description override

- **WHEN** a `[[packages]]` entry specifies `name` and `description`
- **THEN** the package's description in the index is updated after indexing

#### Scenario: Override for nonexistent package

- **WHEN** a `[[packages]]` override references a package name not in the index
- **THEN** a warning is printed to stderr
- **AND** no error occurs
