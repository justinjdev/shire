# MCP Server

## MODIFIED Requirements

### Requirement: search_symbols tool

Full-text search across symbol names and signatures using FTS5.

#### Scenario: Search by keyword

- **WHEN** a search query is provided
- **THEN** results are returned as JSON with name, kind, signature, package, file_path, line, return_type, parameters
- **AND** at most 50 results are returned

#### Scenario: Optional package filter

- **WHEN** a `package` parameter is provided
- **THEN** only symbols from that package are included

#### Scenario: Optional kind filter

- **WHEN** a `kind` parameter is provided
- **THEN** only symbols of that kind are included

### Requirement: get_package_symbols tool

List all symbols within a specific package.

#### Scenario: List symbols

- **WHEN** querying by package name
- **THEN** all symbols for that package are returned as JSON, ordered by file_path then line

#### Scenario: Kind filter

- **WHEN** `kind` is specified
- **THEN** only symbols of that kind are returned

### Requirement: get_symbol tool

Exact name lookup for a symbol.

#### Scenario: Lookup by name

- **WHEN** querying by symbol name
- **THEN** all matching symbols are returned as JSON

#### Scenario: Scoped by package

- **WHEN** `package` is also specified
- **THEN** only symbols from that package are returned
