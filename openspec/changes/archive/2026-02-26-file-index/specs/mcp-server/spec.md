# MCP Server

## MODIFIED Requirements

### Requirement: search_files tool

Full-text search across file paths using FTS5.

#### Scenario: Search by keyword

- **WHEN** a search query is provided
- **THEN** results are returned as JSON with path, package, extension, size_bytes
- **AND** at most 50 results are returned

#### Scenario: Empty query

- **WHEN** the search query is empty or whitespace
- **THEN** a message is returned: "Search query must not be empty"

#### Scenario: Optional package filter

- **WHEN** a `package` parameter is provided
- **THEN** only files belonging to that package are included

#### Scenario: Optional extension filter

- **WHEN** an `extension` parameter is provided (e.g., `proto`, `rs`, `ts`)
- **THEN** only files with that extension are included

#### Scenario: Combined filters

- **WHEN** both `package` and `extension` are provided
- **THEN** only files matching both filters are included

#### Scenario: Special characters in query

- **WHEN** the query contains FTS5 special characters (quotes, operators)
- **THEN** the query is sanitized by wrapping in escaped double quotes
- **AND** no FTS5 syntax error occurs

### Requirement: list_package_files tool

List all files belonging to a specific package.

#### Scenario: List files

- **WHEN** querying by package name
- **THEN** all files for that package are returned as JSON, ordered by path

#### Scenario: Extension filter

- **WHEN** `extension` is specified (e.g., `ts`)
- **THEN** only files with that extension are returned

#### Scenario: Package not found

- **WHEN** querying for a package that has no files
- **THEN** an empty list is returned
