# Custom Package Discovery

## ADDED Requirements

### Requirement: Rule-based directory matching

#### Scenario: All required files present

- **WHEN** a custom rule has `requires = ["main.go", "ownership.yml"]`
- **AND** a directory contains both `main.go` and `ownership.yml`
- **THEN** the directory SHALL be registered as a package

#### Scenario: Missing required file

- **WHEN** a custom rule has `requires = ["main.go", "ownership.yml"]`
- **AND** a directory contains `main.go` but not `ownership.yml`
- **THEN** the directory SHALL NOT be registered as a package

#### Scenario: Glob pattern in requires

- **WHEN** a custom rule has `requires = ["*.proto", "buf.yaml"]`
- **AND** a directory contains `service.proto` and `buf.yaml`
- **THEN** the directory SHALL be registered as a package

#### Scenario: Glob pattern with no match

- **WHEN** a custom rule has `requires = ["*.proto"]`
- **AND** a directory contains no `.proto` files
- **THEN** the directory SHALL NOT be registered as a package

### Requirement: Path scoping

#### Scenario: Paths specified

- **WHEN** a custom rule has `paths = ["services/", "cmd/"]`
- **THEN** only directories under `services/` and `cmd/` SHALL be searched

#### Scenario: No paths specified

- **WHEN** a custom rule has no `paths` field
- **THEN** the entire repository SHALL be searched

#### Scenario: Path does not exist

- **WHEN** a custom rule specifies a path that does not exist in the repo
- **THEN** that path SHALL be silently skipped

### Requirement: Depth limiting

#### Scenario: Max depth set

- **WHEN** a custom rule has `max_depth = 2` and `paths = ["services/"]`
- **THEN** directories SHALL be searched up to 2 levels deep from `services/` (e.g., `services/auth/` matches, `services/auth/internal/handler/` does not)

#### Scenario: No max depth

- **WHEN** a custom rule has no `max_depth` field
- **THEN** directories SHALL be searched to unlimited depth

### Requirement: Rule-specific exclusions

#### Scenario: Exclude directories

- **WHEN** a custom rule has `exclude = ["testdata", "examples"]`
- **THEN** directories named `testdata` or `examples` SHALL be skipped during search for that rule
- **AND** global exclusions from `[discovery].exclude` SHALL also apply

### Requirement: Package naming

#### Scenario: Name prefix

- **WHEN** a custom rule has `name_prefix = "go:"` and matches directory `services/auth`
- **THEN** the package name SHALL be `go:services/auth`

#### Scenario: No name prefix

- **WHEN** a custom rule has no `name_prefix` and matches directory `services/auth`
- **THEN** the package name SHALL be `services/auth`

### Requirement: Nested match prevention

#### Scenario: Subdirectory of a match

- **WHEN** a directory matches a custom rule
- **THEN** subdirectories of that directory SHALL NOT be evaluated as candidates for the same rule

### Requirement: Symbol extraction for custom packages

#### Scenario: Extensions override

- **WHEN** a custom rule has `extensions = [".go"]`
- **THEN** symbol extraction for matched packages SHALL only process files with `.go` extension

#### Scenario: No extensions override

- **WHEN** a custom rule has no `extensions` field
- **THEN** symbol extraction SHALL process all registered extensions (kind-agnostic behavior)

### Requirement: Multiple rules

#### Scenario: Independent rules

- **WHEN** multiple `[[discovery.custom]]` rules are configured
- **THEN** each rule SHALL be evaluated independently
- **AND** a directory MAY be matched by multiple rules (each creates a separate package)

### Requirement: No custom rules configured

#### Scenario: Default behavior

- **WHEN** no `[[discovery.custom]]` rules exist in `shire.toml`
- **THEN** custom discovery SHALL be skipped entirely
- **AND** existing manifest-based discovery SHALL be unaffected
