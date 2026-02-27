# Symbol Extraction

## ADDED Requirements

### Requirement: Perl extraction

#### Scenario: Perl module file dispatch

- **WHEN** a source file has extension `.pm`
- **THEN** it SHALL be dispatched to the Perl symbol extractor

#### Scenario: Perl script file dispatch

- **WHEN** a source file has extension `.pl`
- **THEN** it SHALL be dispatched to the Perl symbol extractor
