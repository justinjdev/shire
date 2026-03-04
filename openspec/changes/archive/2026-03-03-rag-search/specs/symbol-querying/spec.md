## MODIFIED Requirements

### Requirement: search_symbols tool

The `search_symbols` tool SHALL perform hybrid FTS5 + vector search when RAG is enabled.

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
