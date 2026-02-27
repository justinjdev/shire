# Package Discovery

## MODIFIED Requirements

### Requirement: Manifest parser registration

The system SHALL support the following manifest parsers, each detecting packages by their manifest filename:

#### Scenario: Registered parsers

- **WHEN** the build runs
- **THEN** the following manifest parsers SHALL be registered:
  - `package.json` (npm)
  - `go.mod` (Go)
  - `Cargo.toml` (Cargo)
  - `pyproject.toml` (Python)
  - `pom.xml` (Maven)
  - `build.gradle` (Gradle)
  - `build.gradle.kts` (Gradle Kotlin)
  - `cpanfile` (Perl)
  - `Gemfile` (Ruby)
