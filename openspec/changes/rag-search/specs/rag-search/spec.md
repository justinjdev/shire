# RAG Search

## Requirements

### Requirement: Configuration

Parse `[rag]` section from `shire.toml`.

#### Scenario: RAG enabled

- **WHEN** `shire.toml` contains `[rag]` with `enabled = true`
- **THEN** RAG features are active during build and search

#### Scenario: RAG disabled (default)

- **WHEN** `shire.toml` has no `[rag]` section or `enabled = false`
- **THEN** RAG features are skipped entirely
- **AND** no model download or embedding generation occurs

#### Scenario: Custom model

- **WHEN** `[rag]` contains `model = "some-model-name"`
- **THEN** that model is used instead of the default `BAAI/bge-small-en-v1.5`

#### Scenario: Custom cache directory

- **WHEN** `[rag]` contains `cache_dir = "/some/path"`
- **THEN** the embedding model is stored in that directory instead of the default

### Requirement: Embedding generation

Generate vector embeddings for symbols during `shire build`.

#### Scenario: First build with RAG

- **WHEN** `shire build` runs with RAG enabled and no embeddings exist
- **THEN** all symbols are embedded
- **AND** embeddings are stored in the `symbol_embeddings` table
- **AND** progress is reported: "Generating embeddings for N symbols..."

#### Scenario: Incremental build

- **WHEN** a package's source hash has changed
- **THEN** old embeddings for that package's symbols are deleted
- **AND** new embeddings are generated for the updated symbols

#### Scenario: Package removed

- **WHEN** a package is removed from the index
- **THEN** its symbol embeddings are also deleted

#### Scenario: Unchanged package

- **WHEN** a package's source hash has not changed
- **THEN** its embeddings are not regenerated

#### Scenario: Embedding text format

- **WHEN** a symbol is embedded
- **THEN** the text representation is: `{kind} {name} in {package} — {signature} @ {file_path}`
- **AND** if signature is null, it is omitted from the text

### Requirement: Model management

Download and cache the embedding model.

#### Scenario: First run (model not cached)

- **WHEN** RAG is enabled and the model is not yet downloaded
- **THEN** the model is downloaded from Hugging Face Hub
- **AND** a progress message is shown: "Downloading embedding model..."
- **AND** the model is cached for future runs

#### Scenario: Subsequent runs (model cached)

- **WHEN** the model is already cached
- **THEN** no download occurs
- **AND** model loads from the cache directory

#### Scenario: Download failure

- **WHEN** model download fails (no network, HF Hub unreachable)
- **THEN** an error is logged with instructions
- **AND** the build continues without embeddings
- **AND** the build is not considered a failure

### Requirement: Hybrid search

Merge FTS5 and vector search results for `search_symbols`.

#### Scenario: Both FTS5 and vector results exist

- **WHEN** a search query matches symbols via both FTS5 and vector similarity
- **THEN** results are merged using Reciprocal Rank Fusion (RRF)
- **AND** symbols appearing in both result sets rank higher
- **AND** the merged list is sorted by RRF score descending

#### Scenario: Only FTS5 matches

- **WHEN** a query has FTS5 matches but no close vector matches
- **THEN** FTS5 results are returned (RRF with single-source ranking)

#### Scenario: Only vector matches

- **WHEN** a query has no FTS5 keyword matches but has vector similarity matches
- **THEN** vector results are returned
- **AND** this is the key value-add: semantic matches without keyword overlap

#### Scenario: RAG enabled but no embeddings

- **WHEN** RAG is enabled in config but the `symbol_embeddings` table is empty
- **THEN** search falls back to FTS5-only silently

#### Scenario: RAG not compiled

- **WHEN** shire is compiled without the `rag` feature flag
- **THEN** search uses FTS5-only (current behavior)
- **AND** `[rag]` config is ignored

#### Scenario: Vector search failure

- **WHEN** vector search encounters an error at query time
- **THEN** the error is logged
- **AND** FTS5 results are returned as fallback

### Requirement: Storage

Store and query vector embeddings in SQLite via sqlite-vec.

#### Scenario: Schema creation

- **WHEN** the database is opened with RAG enabled
- **THEN** a `symbol_embeddings` virtual table is created using vec0
- **AND** it stores 384-dimensional float vectors keyed by symbol ID

#### Scenario: Insert embeddings

- **WHEN** embeddings are generated for a batch of symbols
- **THEN** they are inserted into `symbol_embeddings` in a transaction

#### Scenario: Delete embeddings

- **WHEN** a package's symbols are re-indexed
- **THEN** old embeddings for those symbol IDs are deleted before new ones are inserted

#### Scenario: Cosine similarity query

- **WHEN** a vector search is performed
- **THEN** the query vector is compared against all stored embeddings using cosine distance
- **AND** the top K most similar results are returned with their distances

### Requirement: Watch daemon integration

Re-embed symbols during incremental rebuilds triggered by the watch daemon.

#### Scenario: File change triggers rebuild

- **WHEN** the watch daemon detects a relevant file change
- **AND** the incremental rebuild updates symbols for a package
- **THEN** embeddings for that package are regenerated

#### Scenario: RAG disabled in watch mode

- **WHEN** RAG is not enabled in config
- **THEN** the watch daemon performs no embedding work
