## 1. Config & Discovery Updates

- [x] 1.1 Add `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts` to `default_manifests()` in `src/config.rs`
- [x] 1.2 Add `.gradle` and `build` to `default_exclude()` in `src/config.rs`
- [x] 1.3 Update `test_default_config` to expect new manifest/exclude counts

## 2. Maven Parser (`src/index/maven.rs`)

- [x] 2.1 Add `quick-xml` with `serialize` feature to `Cargo.toml` dependencies
- [x] 2.2 Create `src/index/maven.rs` with `MavenParser` struct implementing `ManifestParser`
- [x] 2.3 Implement POM XML deserialization — extract `groupId`, `artifactId`, `version`, `description`, `packaging`, `modules`, `parent`, `dependencies`, `dependencyManagement`
- [x] 2.4 Implement `parse()` — return error if no `artifactId`; skip if parent/aggregator POM (`<modules>` + `<packaging>pom</packaging>`); name as `groupId:artifactId`
- [x] 2.5 Implement dependency extraction with scope mapping (compile/absent→runtime, test→dev, provided→peer, runtime/system→runtime)
- [x] 2.6 Add `parse_with_parent_context()` method for inheriting groupId/version from parent POM context
- [x] 2.7 Add unit tests: standard POM, POM without artifactId (error), parent-only POM (skipped), dependencies with scopes, missing version

## 3. Maven Parent POM Context Collection

- [x] 3.1 Add `collect_maven_parent_context()` in `src/index/maven.rs` — walk all `pom.xml` files, collect `groupId:artifactId` → `(groupId, version, dependencyManagement)` for POMs with `<modules>`
- [x] 3.2 Add unit tests for parent context collection

## 4. Gradle Parser (`src/index/gradle.rs`)

- [x] 4.1 Create `src/index/gradle.rs` with `GradleParser` struct implementing `ManifestParser` for `build.gradle`
- [x] 4.2 Create `GradleKtsParser` struct implementing `ManifestParser` for `build.gradle.kts`
- [x] 4.3 Implement `group` extraction via regex (both `group = "..."` and `group = '...'` styles)
- [x] 4.4 Implement `version` extraction via regex
- [x] 4.5 Implement dependency block parsing — extract `implementation`, `api`, `runtimeOnly`, `testImplementation`, `testRuntimeOnly`, `compileOnly`, `testCompileOnly` with string notation (`group:name:version`)
- [x] 4.6 Implement `project(':path')` dependency extraction
- [x] 4.7 Map Gradle configurations to DepKind (implementation/api/runtimeOnly→runtime, testImplementation/testRuntimeOnly→dev, compileOnly/testCompileOnly→peer)
- [x] 4.8 Add `parse_with_settings_context()` method for project name from settings.gradle rootProject.name
- [x] 4.9 Add unit tests: build.gradle with group+version, build.gradle.kts, dependencies with various configs, project deps, no group fallback, unrecognized lines skipped

## 5. Gradle Settings Parser (`src/index/gradle_settings.rs`)

- [x] 5.1 Create `src/index/gradle_settings.rs` with `parse_settings_gradle()` function
- [x] 5.2 Parse `include` directives — extract project paths, convert colon-separated to directory paths (`:lib:core` → `lib/core`)
- [x] 5.3 Parse `rootProject.name` if present
- [x] 5.4 Add unit tests: include directives, rootProject.name, mixed content

## 6. Pipeline Integration (`src/index/mod.rs`)

- [x] 6.1 Add `pub mod maven;` and `pub mod gradle;` and `pub mod gradle_settings;` to `src/index/mod.rs`
- [x] 6.2 Register `MavenParser`, `GradleParser`, `GradleKtsParser` in the `parsers` vec in `build_index()`
- [x] 6.3 Add `collect_maven_parent_context()` call alongside existing Cargo/Go workspace context collection
- [x] 6.4 Add `collect_gradle_settings_context()` call to collect settings.gradle include dirs
- [x] 6.5 Route `pom.xml` through workspace-aware parsing when parent context exists (like Cargo workspace deps)
- [x] 6.6 Annotate Gradle packages with `{"gradle_workspace": true}` metadata when in settings.gradle include list
- [x] 6.7 Handle `settings.gradle`/`settings.gradle.kts` in walk — insert into `manifest_filenames` set like `go.work`, skip during parse phase

## 7. Source Walker Update

- [x] 7.1 Add `"maven" => vec!["java", "kt"]` and `"gradle" => vec!["java", "kt"]` to `extensions_for_kind()` in `src/symbols/walker.rs`
- [x] 7.2 Update `test_extensions_for_kind` test

## 8. Integration Tests

- [x] 8.1 Add Maven integration test — create pom.xml fixture, verify package indexed with correct name and dependencies
- [x] 8.2 Add Maven parent POM test — parent with modules + child inheriting groupId/version
- [x] 8.3 Add Gradle integration test — create build.gradle fixture, verify package indexed with dependencies
- [x] 8.4 Add Gradle settings.gradle test — multi-project build with include directives, verify workspace metadata
- [x] 8.5 Add mixed ecosystem test — repo with both pom.xml and build.gradle, verify both indexed
- [x] 8.6 Verify incremental builds work for new manifest types (content hash change triggers reparse)
