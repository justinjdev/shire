# MCP Server

## Requirements

### Requirement: Server lifecycle

#### Scenario: Start with existing index

- **WHEN** `shire serve` is invoked with a valid index database
- **THEN** the MCP server starts over stdio transport
- **AND** the database is opened in read-only mode

#### Scenario: Start without index

- **WHEN** `shire serve` is invoked and the index database does not exist
- **THEN** the process exits with an error message suggesting `shire build` first

### Requirement: search_packages tool

Full-text search across package name, description, and path using FTS5.

#### Scenario: Search by keyword

- **WHEN** a search query is provided
- **THEN** results are returned as JSON with name, path, kind, version, description, metadata
- **AND** at most 20 results are returned

#### Scenario: Empty query

- **WHEN** the search query is empty or whitespace
- **THEN** a message is returned: "Search query must not be empty"

#### Scenario: Special characters in query

- **WHEN** the query contains FTS5 special characters (quotes, operators)
- **THEN** the query is sanitized by wrapping in escaped double quotes
- **AND** no FTS5 syntax error occurs

### Requirement: get_package tool

Exact name lookup for a single package.

#### Scenario: Package exists

- **WHEN** querying by exact name and the package exists
- **THEN** full package details are returned as JSON

#### Scenario: Package not found

- **WHEN** querying by exact name and no match exists
- **THEN** a message is returned: "Package '<name>' not found"

### Requirement: package_dependencies tool

List dependencies of a given package.

#### Scenario: All dependencies

- **WHEN** `internal_only` is false
- **THEN** all dependencies (internal and external) are returned

#### Scenario: Internal only

- **WHEN** `internal_only` is true
- **THEN** only dependencies where `is_internal = 1` are returned

### Requirement: package_dependents tool

Reverse dependency lookup.

#### Scenario: Find dependents

- **WHEN** querying for dependents of package "B"
- **THEN** all packages that declare a dependency on "B" are returned

### Requirement: dependency_graph tool

Transitive BFS graph traversal.

#### Scenario: Graph query

- **WHEN** querying from a root package with a depth
- **THEN** a list of edges (from, to, dep_kind) is returned
- **AND** depth is clamped to maximum 20

### Requirement: list_packages tool

List all indexed packages.

#### Scenario: Unfiltered

- **WHEN** no kind filter is specified
- **THEN** all packages are returned, ordered by name

#### Scenario: Filtered by kind

- **WHEN** kind is specified (e.g., "npm")
- **THEN** only packages of that kind are returned

### Requirement: index_status tool

#### Scenario: Status query

- **WHEN** the index_status tool is called
- **THEN** `indexed_at`, `git_commit`, and `package_count` are returned from the shire_meta table
