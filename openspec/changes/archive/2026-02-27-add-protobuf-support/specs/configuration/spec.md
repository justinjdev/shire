# Configuration

## ADDED Requirements

### Requirement: Symbol extraction configuration

#### Scenario: Exclude extensions

- **WHEN** `[symbols].exclude_extensions` is specified with a list of extensions (e.g., `[".proto", ".pl"]`)
- **THEN** files with those extensions SHALL be skipped during symbol extraction for all packages

#### Scenario: No symbol config

- **WHEN** `[symbols]` section is not present in `shire.toml`
- **THEN** all registered extensions SHALL be extracted (no exclusions)

#### Scenario: Empty exclude list

- **WHEN** `[symbols].exclude_extensions` is an empty list
- **THEN** all registered extensions SHALL be extracted (same as no config)
