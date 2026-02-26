# Dependency Graph

## Requirements

### Requirement: Internal dependency detection

A dependency is marked `is_internal = 1` when it refers to another package in the same repository.

#### Scenario: Direct name match

- **WHEN** package A declares a dependency on "B"
- **AND** a package named "B" exists in the index
- **THEN** the dependency is marked as internal

#### Scenario: Go module path match

- **WHEN** a Go package declares a dependency on a full module path (e.g., `github.com/company/auth`)
- **AND** a Go package in the index has that module path as its description
- **THEN** the dependency is marked as internal

#### Scenario: External dependency

- **WHEN** a dependency name does not match any indexed package name or Go module path
- **THEN** the dependency is marked as external (`is_internal = 0`)

### Requirement: is_internal recomputation

After any incremental update, `is_internal` is recomputed for all dependencies to reflect the current set of known packages.

#### Scenario: Package added makes dep internal

- **WHEN** package A depends on "B" (currently external)
- **AND** a new build adds package "B" to the index
- **THEN** A's dependency on "B" is updated to `is_internal = 1`

#### Scenario: Package removed makes dep external

- **WHEN** package A depends on "B" (currently internal)
- **AND** package "B" is removed from the index
- **THEN** A's dependency on "B" is updated to `is_internal = 0`

### Requirement: BFS graph traversal

The dependency graph tool performs breadth-first traversal from a root package.

#### Scenario: Transitive dependencies

- **WHEN** querying the graph from package A with depth > 1
- **AND** A depends on B, B depends on C
- **THEN** edges A→B and B→C are both returned

#### Scenario: Depth limiting

- **WHEN** querying with `max_depth = 1`
- **THEN** only direct dependencies of the root are returned

#### Scenario: Internal-only filtering

- **WHEN** `internal_only = true`
- **THEN** only edges where `is_internal = 1` are followed

#### Scenario: Depth cap

- **WHEN** depth exceeds 20
- **THEN** it is clamped to 20 to prevent runaway traversals
