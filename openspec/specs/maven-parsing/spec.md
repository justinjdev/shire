# Maven Parsing

## Requirements

### Requirement: Maven (pom.xml)

#### Scenario: Standard pom.xml with groupId and artifactId

- **WHEN** a `pom.xml` contains `<groupId>`, `<artifactId>`, and `<version>`
- **THEN** a package is created with kind `maven`
- **AND** the package name is `groupId:artifactId` (e.g., `com.example:auth-service`)
- **AND** `<description>` is stored if present

#### Scenario: Dependencies with scopes

- **WHEN** a `pom.xml` contains `<dependencies>` entries
- **THEN** each `<dependency>` is extracted with `groupId:artifactId` as the dep name
- **AND** `<version>` is stored as version_req
- **AND** scope `compile` (or absent) maps to `runtime` dep_kind
- **AND** scope `test` maps to `dev` dep_kind
- **AND** scope `provided` maps to `peer` dep_kind
- **AND** scope `runtime` and `system` map to `runtime` dep_kind

#### Scenario: Missing groupId inherits from parent

- **WHEN** a `pom.xml` has no `<groupId>` but declares a `<parent>` element
- **AND** the parent POM exists in the same repository
- **THEN** `groupId` is inherited from the parent POM

#### Scenario: Missing version inherits from parent

- **WHEN** a `pom.xml` has no `<version>` but declares a `<parent>` element
- **AND** the parent POM exists in the same repository
- **THEN** `version` is inherited from the parent POM

#### Scenario: Parent POM not in repo

- **WHEN** a `pom.xml` references a parent POM that is not found in the repo
- **AND** the child POM does not declare its own `<groupId>`
- **THEN** the package name falls back to directory-based naming (relative path with slashes replaced by hyphens)

#### Scenario: pom.xml without artifactId

- **WHEN** a `pom.xml` has no `<artifactId>`
- **THEN** parsing returns an error
- **AND** the file is recorded as a parse failure

### Requirement: Maven multi-module context

#### Scenario: Parent POM with modules

- **WHEN** a `pom.xml` contains a `<modules>` section with `<module>` entries
- **THEN** the parent POM is NOT indexed as a package itself (similar to Cargo workspace root without `[package]`)
- **AND** its `groupId` and `version` are collected as workspace context for child resolution

#### Scenario: POM with both packaging=pom and artifactId

- **WHEN** a `pom.xml` has `<packaging>pom</packaging>` and `<modules>`
- **THEN** it is treated as a parent/aggregator POM
- **AND** it is NOT indexed as a package

#### Scenario: POM with modules AND its own code

- **WHEN** a `pom.xml` has `<modules>` but also has `<packaging>` other than `pom` (e.g., `jar`)
- **THEN** it IS indexed as a package (it has its own artifact)
- **AND** its context is still used for child resolution

### Requirement: Maven dependency version from parent dependencyManagement

#### Scenario: Version from dependencyManagement

- **WHEN** a child `pom.xml` declares a dependency without `<version>`
- **AND** the parent POM defines that dependency in `<dependencyManagement>`
- **THEN** the version is resolved from the parent's `dependencyManagement`

#### Scenario: Version not in dependencyManagement

- **WHEN** a child `pom.xml` declares a dependency without `<version>`
- **AND** the dependency is not in any parent's `dependencyManagement`
- **THEN** the dependency is recorded with `version_req: None`
