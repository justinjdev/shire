# Manifest Parsing

## Requirements

### Requirement: npm (package.json)

#### Scenario: Standard package.json

- **WHEN** a `package.json` contains `name`, `version`, and `description`
- **THEN** a package is created with kind `npm`
- **AND** `dependencies` are extracted as `runtime` deps
- **AND** `devDependencies` are extracted as `dev` deps
- **AND** `peerDependencies` are extracted as `peer` deps

#### Scenario: Missing name

- **WHEN** `package.json` has no `name` field
- **THEN** the package name is derived from the relative directory path (slashes replaced with hyphens)

#### Scenario: Workspace protocol version

- **WHEN** a dependency version starts with `workspace:` (e.g., `workspace:*`, `workspace:^`, `workspace:~1.0.0`)
- **THEN** the `workspace:` prefix is stripped from the stored version_req
- **AND** `*` becomes `*`, `^` becomes `^`, `~1.0.0` becomes `~1.0.0`

### Requirement: Go (go.mod)

#### Scenario: Standard go.mod

- **WHEN** a `go.mod` contains a `module` directive
- **THEN** a package is created with kind `go`
- **AND** the package name is the last segment of the module path
- **AND** the full module path is stored in `description` (used for internal dep resolution)
- **AND** the Go version is stored as the package version
- **AND** `require` directives are extracted as `runtime` deps with full module paths as dep names

#### Scenario: Multi-line require block

- **WHEN** `go.mod` uses `require ( ... )` syntax
- **THEN** each line within the block is parsed as a dependency
- **AND** comment-only lines are skipped
- **AND** inline comments are stripped

### Requirement: Cargo (Cargo.toml)

#### Scenario: Standard Cargo.toml with [package]

- **WHEN** a `Cargo.toml` contains a `[package]` section
- **THEN** a package is created with kind `cargo`
- **AND** `[dependencies]` are extracted as `runtime` deps
- **AND** `[dev-dependencies]` are extracted as `dev` deps
- **AND** `[build-dependencies]` are extracted as `build` deps

#### Scenario: Workspace Cargo.toml without [package]

- **WHEN** a `Cargo.toml` has no `[package]` section (e.g., workspace root)
- **THEN** parsing returns an error
- **AND** the file is recorded as a parse failure (not indexed as a package)

#### Scenario: Workspace root Cargo.toml

- **WHEN** a `Cargo.toml` contains `[workspace]` but no `[package]`
- **THEN** it is not indexed as a package
- **AND** its `[workspace.dependencies]` are collected as workspace context

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

#### Scenario: Table-style dependency

- **WHEN** a dependency is specified as a table (e.g., `tokio = { version = "1", features = ["full"] }`)
- **THEN** the version is extracted from the `version` key within the table

### Requirement: Python (pyproject.toml)

#### Scenario: Standard pyproject.toml with [project]

- **WHEN** a `pyproject.toml` contains a `[project]` section
- **THEN** a package is created with kind `python`
- **AND** PEP 508 dependency strings are parsed to extract package name and version requirement

#### Scenario: Missing [project] section

- **WHEN** a `pyproject.toml` has no `[project]` section
- **THEN** the package name is derived from the relative directory path
