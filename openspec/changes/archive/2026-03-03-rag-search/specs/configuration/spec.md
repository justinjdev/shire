## MODIFIED Requirements

### Requirement: RAG configuration

The system SHALL parse a `[rag]` section from `shire.toml`.

#### Scenario: RAG enabled

- **WHEN** `shire.toml` contains `[rag]` with `enabled = true`
- **THEN** RAG features SHALL be active during build and search

#### Scenario: RAG disabled (default)

- **WHEN** `shire.toml` has no `[rag]` section or `enabled = false`
- **THEN** RAG features SHALL be skipped entirely
- **AND** no model download or embedding generation SHALL occur

#### Scenario: Custom model

- **WHEN** `[rag]` contains `model = "some-model-name"`
- **THEN** that model SHALL be used instead of the default

#### Scenario: Custom cache directory

- **WHEN** `[rag]` contains `cache_dir = "/some/path"`
- **THEN** the embedding model SHALL be stored in that directory
