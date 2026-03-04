## ADDED Requirements

### Requirement: Init --no-hook flag

The `shire init` and `shire init --global` commands SHALL accept a `--no-hook` flag that skips PostToolUse hook installation and configures the MCP server for on-demand reindexing.

#### Scenario: Local init with --no-hook

- **WHEN** `shire init --no-hook` is invoked
- **THEN** `shire.toml` SHALL be created (or skipped if exists)
- **AND** `.claude/settings.local.json` SHALL be patched with `mcpServers.shire` using args `["serve", "--root", "."]`
- **AND** no PostToolUse hook SHALL be added
- **AND** next-step instructions SHALL reflect the no-hook setup

#### Scenario: Global init with --no-hook

- **WHEN** `shire init --global --no-hook` is invoked
- **THEN** `~/.claude/shire.toml` SHALL be created (or skipped if exists)
- **AND** `~/.claude/settings.json` SHALL be patched with `mcpServers.shire` using args `["serve", "--root", "."]`
- **AND** no PostToolUse hook SHALL be added

#### Scenario: Init without --no-hook (default)

- **WHEN** `shire init` is invoked without `--no-hook`
- **THEN** behavior SHALL be unchanged from current implementation
- **AND** PostToolUse hook SHALL be installed as before

#### Scenario: Idempotent --no-hook

- **WHEN** `shire init --no-hook` is run twice
- **THEN** the MCP server entry SHALL not be duplicated
- **AND** no PostToolUse hook SHALL be present
