# Configuration

## MODIFIED Requirements

### Requirement: Discovery configuration

#### Scenario: Default manifests

- **WHEN** `[discovery].manifests` is not specified
- **THEN** defaults to: `package.json`, `go.mod`, `go.work`, `Cargo.toml`, `pyproject.toml`, `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts`, `cpanfile`
