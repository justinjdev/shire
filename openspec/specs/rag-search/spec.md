# RAG Search

## Requirements

### Requirement: Hybrid search

The system SHALL merge FTS5 and vector search results for `search_symbols` using Reciprocal Rank Fusion.

#### Scenario: Both FTS5 and vector results exist

- **WHEN** a search query matches symbols via both FTS5 and vector similarity
- **THEN** results SHALL be merged using RRF with k=60
- **AND** symbols appearing in both result sets SHALL rank higher
- **AND** the merged list SHALL be sorted by RRF score descending

#### Scenario: Only FTS5 matches

- **WHEN** a query has FTS5 matches but no close vector matches
- **THEN** FTS5 results SHALL be returned with single-source RRF ranking

#### Scenario: Only vector matches

- **WHEN** a query has no FTS5 keyword matches but has vector similarity matches
- **THEN** vector results SHALL be returned

#### Scenario: RAG enabled but no embeddings

- **WHEN** RAG is enabled in config but the `symbol_embeddings` table is empty
- **THEN** search SHALL fall back to FTS5-only silently

#### Scenario: RAG not compiled

- **WHEN** shire is compiled without the `rag` feature flag
- **THEN** search SHALL use FTS5-only (current behavior)
- **AND** `[rag]` config SHALL be ignored

#### Scenario: Vector search failure

- **WHEN** vector search encounters an error at query time
- **THEN** the error SHALL be logged
- **AND** FTS5 results SHALL be returned as fallback

### Requirement: Cosine similarity query

The system SHALL support KNN queries over stored embeddings using cosine distance.

#### Scenario: Query by vector

- **WHEN** a vector search is performed with a query embedding
- **THEN** the query SHALL be compared against all stored embeddings using cosine distance
- **AND** the top K most similar results SHALL be returned with their distances
