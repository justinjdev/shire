# Configuration

## ADDED Requirements

### Requirement: Custom discovery configuration

#### Scenario: Custom rule fields

- **WHEN** a `[[discovery.custom]]` entry is specified in `shire.toml`
- **THEN** it SHALL accept the following fields:
  - `name` (required): rule identifier string
  - `kind` (required): package kind string (e.g., `"go"`, `"proto"`, `"gradle"`)
  - `requires` (required): list of filename patterns (glob-supported) that must all be present
  - `paths` (optional): list of directory paths to scope the search
  - `exclude` (optional): list of directory names to skip
  - `max_depth` (optional): maximum directory depth to search
  - `name_prefix` (optional): string to prepend to directory-derived package names
  - `extensions` (optional): list of file extensions for symbol extraction override

#### Scenario: Invalid custom rule

- **WHEN** a `[[discovery.custom]]` entry is missing `name`, `kind`, or `requires`
- **THEN** a configuration error SHALL be reported
- **AND** the build SHALL NOT proceed
