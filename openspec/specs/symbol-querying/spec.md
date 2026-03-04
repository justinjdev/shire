# Symbol Querying

## Requirements

### Requirement: search_symbols tool

Full-text search across symbol names and signatures. Performs hybrid FTS5 + vector search when RAG is enabled.

#### Scenario: Search by name

- **WHEN** searching for "processPayment"
- **THEN** results include symbols whose name matches
- **AND** each result includes: name, kind, signature, package, file_path, line, return_type, parameters

#### Scenario: Search by signature content

- **WHEN** searching for "Receipt"
- **THEN** results include symbols whose signature or return type contains "Receipt"

#### Scenario: Filter by package

- **WHEN** searching with an optional `package` filter
- **THEN** only symbols from that package are returned

#### Scenario: Filter by kind

- **WHEN** searching with an optional `kind` filter (e.g., "function", "class", "struct")
- **THEN** only symbols of that kind are returned

#### Scenario: Result limit

- **WHEN** more than 50 symbols match
- **THEN** at most 50 results are returned

#### Scenario: Empty query

- **WHEN** the search query is empty or whitespace
- **THEN** a message is returned: "Search query must not be empty"

#### Scenario: Hybrid search active

- **WHEN** RAG is enabled and embeddings exist
- **THEN** `search_symbols` SHALL run both FTS5 keyword search and vector similarity search
- **AND** results SHALL be merged using Reciprocal Rank Fusion
- **AND** the response format SHALL remain unchanged (name, kind, signature, package, file_path, line, return_type, parameters)

#### Scenario: RAG disabled or unavailable

- **WHEN** RAG is not enabled, not compiled, or embeddings are empty
- **THEN** `search_symbols` SHALL use FTS5-only search (current behavior)
- **AND** no error SHALL be raised

#### Scenario: Filters with hybrid search

- **WHEN** searching with `package` or `kind` filters and RAG is enabled
- **THEN** filters SHALL apply to both FTS5 and vector result sets before merging

### Requirement: get_package_symbols tool

List all symbols within a specific package.

#### Scenario: All symbols

- **WHEN** querying symbols for package "auth-service" with no kind filter
- **THEN** all symbols from that package are returned, ordered by file_path then line

#### Scenario: Filtered by kind

- **WHEN** querying with kind "function"
- **THEN** only functions are returned

#### Scenario: Package not found

- **WHEN** the package name does not exist
- **THEN** a message is returned: "Package '<name>' not found"

#### Scenario: Package has no symbols

- **WHEN** the package exists but has no extracted symbols
- **THEN** an empty list is returned

### Requirement: get_symbol tool

Exact name lookup for a symbol, optionally scoped to a package.

#### Scenario: Unique symbol name

- **WHEN** searching for symbol "AuthService" and it exists in one package
- **THEN** full symbol details are returned including signature, parameters, return_type, file_path, line

#### Scenario: Symbol name exists in multiple packages

- **WHEN** "Handler" exists in multiple packages and no package filter is specified
- **THEN** all matching symbols are returned

#### Scenario: Symbol scoped to package

- **WHEN** searching for "Handler" with package "auth-service"
- **THEN** only the symbol from that package is returned

#### Scenario: Symbol not found

- **WHEN** the symbol name does not exist
- **THEN** a message is returned: "Symbol '<name>' not found"
