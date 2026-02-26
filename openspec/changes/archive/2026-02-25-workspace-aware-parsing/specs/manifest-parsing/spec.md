# Manifest Parsing

## MODIFIED Requirements

### Requirement: Cargo (Cargo.toml)

#### Scenario: Workspace-inherited dependency

- **WHEN** a Cargo member's dependency uses `workspace = true` (e.g., `tokio = { workspace = true }`)
- **AND** the workspace root's `[workspace.dependencies]` defines `tokio = { version = "1" }`
- **THEN** the dependency's version_req is resolved to "1"

#### Scenario: Workspace-inherited dependency with local override

- **WHEN** a Cargo member's dependency uses `workspace = true` with additional keys (e.g., `tokio = { workspace = true, features = ["full"] }`)
- **THEN** the version is still resolved from `[workspace.dependencies]`
- **AND** local keys like `features` are ignored (only version matters for indexing)

#### Scenario: Workspace dependency not found in root

- **WHEN** a Cargo member uses `workspace = true` for dep "foo"
- **AND** "foo" is not in `[workspace.dependencies]`
- **THEN** the dependency is recorded with `version_req: None`

#### Scenario: Workspace root Cargo.toml

- **WHEN** a `Cargo.toml` contains `[workspace]` but no `[package]`
- **THEN** it is not indexed as a package (existing behavior unchanged)
- **AND** its `[workspace.dependencies]` are collected as workspace context

### Requirement: npm (package.json)

#### Scenario: Workspace protocol version

- **WHEN** a dependency version starts with `workspace:` (e.g., `workspace:*`, `workspace:^`, `workspace:~1.0.0`)
- **THEN** the `workspace:` prefix is stripped from the stored version_req
- **AND** `*` becomes `*`, `^` becomes `^`, `~1.0.0` becomes `~1.0.0`
