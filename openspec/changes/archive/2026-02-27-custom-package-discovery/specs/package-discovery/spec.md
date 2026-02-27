# Package Discovery

## ADDED Requirements

### Requirement: Custom discovery phase

#### Scenario: Custom rules present

- **WHEN** `[[discovery.custom]]` rules are configured
- **THEN** custom discovery SHALL run after the manifest walk phase
- **AND** discovered packages SHALL be inserted into the same pipeline as manifest-discovered packages

#### Scenario: Deduplication with manifest discovery

- **WHEN** a custom rule matches a directory already discovered by a manifest parser
- **THEN** the custom rule's package SHALL take precedence (last-writer-wins by path)
