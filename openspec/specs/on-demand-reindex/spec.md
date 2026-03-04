# On-Demand Reindex

## Requirements

### Requirement: Staleness detection

The MCP server SHALL detect when the index is stale before answering tool calls. Staleness is determined by comparing the `.git/index` file mtime against the `indexed_at` timestamp stored in `shire_meta`.

#### Scenario: Index is fresh

- **WHEN** the MCP server receives a tool call
- **AND** the `.git/index` mtime is older than or equal to the stored `indexed_at` timestamp
- **THEN** no rebuild SHALL occur
- **AND** the tool call SHALL proceed immediately

#### Scenario: Index is stale

- **WHEN** the MCP server receives a tool call
- **AND** the `.git/index` mtime is newer than the stored `indexed_at` timestamp
- **THEN** a rebuild SHALL be triggered before answering the tool call

#### Scenario: No git directory

- **WHEN** the MCP server receives a tool call
- **AND** no `.git/index` file exists at the repo root
- **THEN** no staleness check SHALL occur
- **AND** the tool call SHALL proceed with the existing index

#### Scenario: No existing index

- **WHEN** the MCP server receives a tool call
- **AND** the index database does not exist
- **THEN** a full build SHALL be triggered before answering the tool call

#### Scenario: On-demand mode not configured

- **WHEN** the MCP server was started without a `--root` flag
- **THEN** staleness detection SHALL be disabled
- **AND** all tool calls SHALL proceed immediately (read-only mode)

### Requirement: On-demand rebuild

The MCP server SHALL rebuild the index synchronously before answering a tool call when staleness is detected.

#### Scenario: Successful rebuild

- **WHEN** staleness is detected
- **THEN** the server SHALL call the incremental build with progress bars suppressed
- **AND** the database connection SHALL be reopened after the rebuild completes
- **AND** the tool call SHALL proceed with the fresh index

#### Scenario: Rebuild failure

- **WHEN** a rebuild is triggered and fails
- **THEN** the server SHALL log the error via MCP logging notification
- **AND** the tool call SHALL proceed with the existing (stale) index
- **AND** the server SHALL NOT crash

#### Scenario: Concurrent tool calls during rebuild

- **WHEN** a rebuild is in progress and another tool call arrives
- **THEN** the second tool call SHALL wait for the rebuild to complete
- **AND** the second tool call SHALL NOT trigger a duplicate rebuild

### Requirement: MCP logging during rebuild

The MCP server SHALL send logging notifications during on-demand rebuilds so the client can display rebuild status.

#### Scenario: Rebuild starts

- **WHEN** a rebuild is triggered
- **THEN** the server SHALL send an MCP logging notification with level `Info` and message indicating rebuild has started

#### Scenario: Rebuild completes

- **WHEN** a rebuild completes successfully
- **THEN** the server SHALL send an MCP logging notification with level `Info` and message indicating rebuild is complete
