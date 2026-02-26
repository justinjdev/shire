# Gradle Parsing

## Requirements

### Requirement: Gradle (build.gradle / build.gradle.kts)

#### Scenario: Standard build.gradle with group and version

- **WHEN** a `build.gradle` or `build.gradle.kts` contains `group` and `version` assignments
- **THEN** a package is created with kind `gradle`
- **AND** the package name is `group:projectName` where projectName is derived from the directory name or settings.gradle
- **AND** both single-quoted and double-quoted string values are recognized

#### Scenario: build.gradle without group

- **WHEN** a `build.gradle` has no `group` assignment
- **THEN** the package name falls back to directory-based naming (relative path with slashes replaced by hyphens)

#### Scenario: Kotlin DSL build.gradle.kts

- **WHEN** a `build.gradle.kts` is parsed
- **THEN** it follows the same extraction rules as `build.gradle`
- **AND** both `=` assignment and Kotlin property syntax are handled

#### Scenario: Dependencies block — string notation

- **WHEN** a `build.gradle` contains dependencies like `implementation 'com.example:lib:1.0'` or `implementation("com.example:lib:1.0")`
- **THEN** the dependency is extracted with `com.example:lib` as the dep name and `1.0` as version_req

#### Scenario: Dependencies block — configuration mapping

- **WHEN** dependencies are declared with Gradle configurations
- **THEN** `implementation`, `api`, `runtimeOnly` map to `runtime` dep_kind
- **AND** `testImplementation`, `testRuntimeOnly` map to `dev` dep_kind
- **AND** `compileOnly`, `testCompileOnly` map to `peer` dep_kind

#### Scenario: Dependencies without version

- **WHEN** a dependency is declared as `implementation 'com.example:lib'` (no version segment)
- **THEN** the dependency is extracted with `com.example:lib` as the dep name and `version_req: None`

#### Scenario: Project dependencies

- **WHEN** a dependency is declared as `implementation project(':submodule')` or `implementation(project(":submodule"))`
- **THEN** the dependency is extracted with the project path as the dep name (e.g., `:submodule`)
- **AND** `version_req` is None

#### Scenario: Unrecognized dependency format

- **WHEN** a dependency line does not match known patterns (e.g., dynamic computation)
- **THEN** the line is silently skipped
- **AND** no error is raised

### Requirement: Gradle settings workspace context

`settings.gradle` and `settings.gradle.kts` provide multi-project structure via `include` directives. They are NOT indexed as packages.

#### Scenario: settings.gradle with include directives

- **WHEN** a `settings.gradle` or `settings.gradle.kts` contains `include` statements
- **THEN** the included project paths are extracted (e.g., `include ':app', ':lib:core'`)
- **AND** colon-separated paths are converted to directory paths (`:lib:core` → `lib/core`)
- **AND** paths are resolved relative to the settings file location

#### Scenario: Gradle workspace member annotation

- **WHEN** a `build.gradle` or `build.gradle.kts` package exists in a directory listed in `settings.gradle` `include` directives
- **THEN** the package's metadata includes `{"gradle_workspace": true}`

#### Scenario: settings.gradle not indexed as package

- **WHEN** a `settings.gradle` or `settings.gradle.kts` file is discovered
- **THEN** it is NOT indexed as a package
- **AND** it is only used for workspace context

#### Scenario: rootProject.name in settings

- **WHEN** `settings.gradle` contains `rootProject.name = 'my-project'`
- **THEN** the root project name is available for build.gradle files in the same directory to use as their project name
