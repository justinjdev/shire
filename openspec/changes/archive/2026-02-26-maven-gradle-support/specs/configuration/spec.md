## MODIFIED Requirements

### Requirement: Discovery configuration

#### Scenario: Custom manifest list

- **WHEN** `[discovery].manifests` is specified
- **THEN** only listed manifest filenames are discovered during walk

#### Scenario: Custom exclude list

- **WHEN** `[discovery].exclude` is specified
- **THEN** listed directory names are skipped during walk

#### Scenario: Default manifests

- **WHEN** `[discovery].manifests` is not specified
- **THEN** defaults to: `package.json`, `go.mod`, `go.work`, `Cargo.toml`, `pyproject.toml`, `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts`
