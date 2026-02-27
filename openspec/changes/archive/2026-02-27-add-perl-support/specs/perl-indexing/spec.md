# Perl Indexing

## ADDED Requirements

### Requirement: cpanfile package discovery

#### Scenario: Simple cpanfile

- **WHEN** a directory contains a `cpanfile` with `requires 'DBI', '>= 1.600';`
- **THEN** a package SHALL be created with:
  - name: directory path relative to repo root
  - kind: `perl`
  - dependency: `DBI` with version_req `>= 1.600` and dep_kind `Runtime`

#### Scenario: cpanfile with unversioned dependency

- **WHEN** a `cpanfile` contains `requires 'JSON::XS';`
- **THEN** a dependency SHALL be created with name `JSON::XS`, no version_req, and dep_kind `Runtime`

#### Scenario: cpanfile with test dependencies

- **WHEN** a `cpanfile` contains `on 'test' => sub { requires 'Test::More', '0.88'; };`
- **THEN** a dependency SHALL be created with name `Test::More`, version_req `0.88`, and dep_kind `Dev`

#### Scenario: cpanfile with dynamic Perl code

- **WHEN** a `cpanfile` contains Perl expressions beyond simple `requires` statements
- **THEN** unparseable lines SHALL be silently skipped
- **AND** successfully parsed dependencies SHALL still be extracted

### Requirement: Perl symbol extraction

#### Scenario: Top-level subroutine

- **WHEN** a `.pm` file contains `sub process_payment { ... }` outside any `package` declaration
- **THEN** a symbol SHALL be extracted with:
  - name: `process_payment`
  - kind: `function`
  - visibility: `public`

#### Scenario: Subroutine in a package

- **WHEN** a `.pm` file contains `sub validate { ... }` inside `package Auth::Service;`
- **THEN** a symbol SHALL be extracted with:
  - name: `validate`
  - kind: `method`
  - parent_symbol: `Auth::Service`

#### Scenario: Package declaration

- **WHEN** a `.pm` file contains `package Payment::Handler;`
- **THEN** a symbol SHALL be extracted with:
  - name: `Payment::Handler`
  - kind: `class`

#### Scenario: Private subroutine

- **WHEN** a subroutine name starts with `_` (e.g., `sub _internal_helper { ... }`)
- **THEN** it SHALL NOT be extracted

#### Scenario: Perl script files

- **WHEN** a `.pl` file contains subroutine and package definitions
- **THEN** symbols SHALL be extracted using the same rules as `.pm` files
