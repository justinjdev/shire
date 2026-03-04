# RAG Embedding

## Requirements

### Requirement: Embedding generation

The system SHALL generate vector embeddings for symbols during `shire build` when RAG is enabled.

#### Scenario: First build with RAG

- **WHEN** `shire build` runs with RAG enabled and no embeddings exist
- **THEN** all symbols SHALL be embedded
- **AND** embeddings SHALL be stored in the `symbol_embeddings` table
- **AND** progress SHALL be reported to stderr

#### Scenario: Incremental build

- **WHEN** a package's source hash has changed
- **THEN** old embeddings for that package's symbols SHALL be deleted
- **AND** new embeddings SHALL be generated for the updated symbols

#### Scenario: Package removed

- **WHEN** a package is removed from the index
- **THEN** its symbol embeddings SHALL also be deleted

#### Scenario: Unchanged package

- **WHEN** a package's source hash has not changed
- **THEN** its embeddings SHALL NOT be regenerated

#### Scenario: Embedding text format

- **WHEN** a symbol is embedded
- **THEN** the text representation SHALL be: `{kind} {name} in {package} — {signature} @ {file_path}`
- **AND** if signature is null, it SHALL be omitted from the text

### Requirement: Model management

The system SHALL download and cache the embedding model automatically.

#### Scenario: First run (model not cached)

- **WHEN** RAG is enabled and the model is not yet downloaded
- **THEN** the model SHALL be downloaded from Hugging Face Hub
- **AND** a progress message SHALL be shown
- **AND** the model SHALL be cached for future runs

#### Scenario: Subsequent runs (model cached)

- **WHEN** the model is already cached
- **THEN** no download SHALL occur
- **AND** model SHALL load from the cache directory

#### Scenario: Download failure

- **WHEN** model download fails
- **THEN** an error SHALL be logged
- **AND** the build SHALL continue without embeddings
- **AND** the build SHALL NOT be considered a failure

### Requirement: Vector storage

The system SHALL store embeddings in SQLite via the sqlite-vec extension.

#### Scenario: Schema creation

- **WHEN** the database is opened with RAG enabled
- **THEN** a `symbol_embeddings` virtual table SHALL be created using vec0
- **AND** it SHALL store 384-dimensional float vectors with cosine distance metric

#### Scenario: Insert embeddings

- **WHEN** embeddings are generated for a batch of symbols
- **THEN** they SHALL be inserted into `symbol_embeddings`

#### Scenario: Delete embeddings

- **WHEN** a package's symbols are re-indexed
- **THEN** old embeddings for those symbol IDs SHALL be deleted before new ones are inserted

### Requirement: Watch daemon integration

The system SHALL re-embed symbols during incremental rebuilds triggered by the watch daemon.

#### Scenario: File change triggers rebuild with RAG

- **WHEN** the watch daemon triggers a rebuild and symbols change
- **THEN** embeddings for changed packages SHALL be regenerated

#### Scenario: RAG disabled in watch mode

- **WHEN** RAG is not enabled in config
- **THEN** the watch daemon SHALL perform no embedding work
