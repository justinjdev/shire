## MODIFIED Requirements

### Requirement: Server lifecycle

#### Scenario: Start with existing index

- **WHEN** `shire serve` is invoked with a valid index database
- **THEN** the MCP server starts over stdio transport
- **AND** the database is opened in read-only mode

#### Scenario: Start with existing index in on-demand mode

- **WHEN** `shire serve --root <path>` is invoked with a valid index database
- **THEN** the MCP server starts over stdio transport
- **AND** the database is opened in read-only mode
- **AND** on-demand reindexing SHALL be enabled

#### Scenario: Start without index in on-demand mode

- **WHEN** `shire serve --root <path>` is invoked and the index database does not exist
- **THEN** the MCP server SHALL start normally
- **AND** the first tool call SHALL trigger a full build before answering

#### Scenario: Start without index

- **WHEN** `shire serve` is invoked without `--root` and the index database does not exist
- **THEN** the process exits with an error message suggesting `shire build` first

## ADDED Requirements

### Requirement: Serve command --root flag

The `shire serve` subcommand SHALL accept an optional `--root` flag specifying the repository root directory.

#### Scenario: Root flag provided

- **WHEN** `shire serve --root <path>` is invoked
- **THEN** the server SHALL store the repo root, resolved config, and db path
- **AND** on-demand reindexing SHALL be enabled

#### Scenario: Root flag omitted

- **WHEN** `shire serve` is invoked without `--root`
- **THEN** the server SHALL operate in read-only mode
- **AND** no rebuild capability SHALL be available
