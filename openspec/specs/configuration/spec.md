# Configuration

## Requirements

### Requirement: Config file loading

#### Scenario: shire.toml present

- **WHEN** `shire.toml` exists in the repository root
- **THEN** it is parsed and its values override defaults

#### Scenario: No config file

- **WHEN** no `shire.toml` exists
- **THEN** default configuration is used

### Requirement: Discovery configuration

#### Scenario: Custom manifest list

- **WHEN** `[discovery].manifests` is specified
- **THEN** only listed manifest filenames are discovered during walk

#### Scenario: Custom exclude list

- **WHEN** `[discovery].exclude` is specified
- **THEN** listed directory names are skipped during walk

#### Scenario: Default manifests

- **WHEN** `[discovery].manifests` is not specified
- **THEN** defaults to: `package.json`, `go.mod`, `go.work`, `Cargo.toml`, `pyproject.toml`, `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts`

### Requirement: Package overrides

#### Scenario: Description override

- **WHEN** a `[[packages]]` entry specifies `name` and `description`
- **THEN** the package's description in the index is updated after indexing

#### Scenario: Override for nonexistent package

- **WHEN** a `[[packages]]` override references a package name not in the index
- **THEN** a warning is printed to stderr
- **AND** no error occurs

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
