## Context

Shire currently supports 4 ecosystems (npm, Go, Cargo, Python) via the `ManifestParser` trait. Each parser implements `filename()` and `parse()`, returning a `PackageInfo` with dependencies. Workspace-level context is handled via separate collection passes (Cargo workspace deps, go.work dirs) before parsing.

Maven and Gradle are the two dominant JVM build systems. Maven uses XML (`pom.xml`), Gradle uses Groovy/Kotlin DSL (`build.gradle`/`build.gradle.kts`). Both support multi-module projects. Adding these follows the same patterns already established for other ecosystems.

## Goals / Non-Goals

**Goals:**
- Parse `pom.xml` to extract Maven artifact coordinates, dependencies with scopes
- Parse `build.gradle` and `build.gradle.kts` to extract project name, version, dependencies
- Parse `settings.gradle` / `settings.gradle.kts` for multi-project workspace context (like `go.work`)
- Handle Maven parent POM inheritance for version/groupId within the same repo
- Register `.java` and `.kt` extensions for source file walking (so file index covers them)
- Incremental indexing works automatically (content hashing already covers any manifest file)

**Non-Goals:**
- Tree-sitter symbol extraction for Java/Kotlin (future change)
- Evaluating Gradle build scripts (no Groovy/Kotlin evaluation — regex only)
- Resolving dependencies from remote Maven repositories or Gradle plugin portals
- Supporting Gradle version catalogs (`libs.versions.toml`) — can be added later
- Supporting Maven BOM imports

## Decisions

### 1. Maven XML parsing: `quick-xml` crate

Maven's `pom.xml` is XML. We need a lightweight XML parser. Options:
- **`quick-xml`**: Fast, minimal, event-based or serde-based. Already commonly used in Rust.
- **`roxmltree`**: Read-only DOM tree, simple API but allocates full tree.
- **Manual regex**: Fragile for XML.

**Decision**: Use `quick-xml` with serde deserialization into typed structs. It's fast, well-maintained, and the POM structure is regular enough for direct deserialization. Only deserialize the fields we need (`groupId`, `artifactId`, `version`, `description`, `parent`, `dependencies`, `modules`).

### 2. Gradle parsing: regex-based extraction

Gradle build files are Groovy or Kotlin scripts. Evaluating them requires a JVM runtime. Options:
- **Regex extraction**: Parse `dependencies { }` blocks, `group`/`version` assignments. Works for ~80% of real-world Gradle files.
- **Shelling out to `gradle dependencies`**: Accurate but requires Gradle installation, extremely slow (JVM startup), and breaks in CI.
- **Tree-sitter Groovy/Kotlin**: Overkill for extracting a few string literals.

**Decision**: Regex-based extraction. Accept that dynamic/conditional builds won't be fully captured. This matches shire's philosophy of fast, best-effort indexing. Focus on the common patterns:
- `group = "com.example"` / `group = 'com.example'`
- `version = "1.0"` / `version = '1.0'`
- `implementation("com.example:lib:1.0")` / `implementation 'com.example:lib:1.0'`
- `testImplementation`, `compileOnly`, `runtimeOnly`, `api` configurations

### 3. Maven package naming: `groupId:artifactId`

Maven uniquely identifies packages by `groupId:artifactId`. Options:
- Use `groupId:artifactId` as the package name (e.g., `com.example:auth-service`)
- Use just `artifactId` (e.g., `auth-service`)

**Decision**: Use `groupId:artifactId`. This avoids collisions (multiple modules can have the same artifactId) and is the canonical Maven identifier. It also makes internal dependency resolution straightforward — a dependency on `com.example:auth-service` matches the package named `com.example:auth-service`.

### 4. Gradle package naming: directory-based with optional group

Gradle project names default to the directory name. The `group` property in `build.gradle` may or may not be set. Options:
- Always use `group:name` (fails when group is absent)
- Use `group:name` when group exists, directory name otherwise
- Always use directory name

**Decision**: Use `group:name` when both are available, fall back to directory-based name (same as other parsers). Store the project name from `settings.gradle` if available. This gives reasonable names without requiring group to be set.

### 5. Settings.gradle as workspace context (not a package)

`settings.gradle` defines the multi-project structure via `include` directives. Like `go.work`, it provides context rather than being a package itself.

**Decision**: Follow the `go.work` pattern exactly — parse `settings.gradle` for workspace member directories, annotate Gradle packages with `{"gradle_workspace": true}` metadata when they appear as included subprojects. The settings file itself is NOT indexed as a package.

### 6. Maven parent POM inheritance

Maven modules often inherit `groupId` and `version` from a parent POM. The parent may be in the same repo.

**Decision**: Collect parent POM context in a separate pass (like Cargo workspace deps). Walk all `pom.xml` files, build a map of `groupId:artifactId` → `(groupId, version)`. When parsing a child POM that references a `<parent>`, look up inherited values from this map. If the parent isn't in the repo, use whatever the child declares directly.

### 7. Dependency scope mapping

Maven has scopes: `compile` (default), `test`, `provided`, `runtime`, `system`. Gradle has configurations: `implementation`, `api`, `compileOnly`, `runtimeOnly`, `testImplementation`, `testCompileOnly`, `testRuntimeOnly`.

**Decision**: Map to existing `DepKind` variants:
- Maven: `compile`/`runtime` → Runtime, `test` → Dev, `provided` → Peer, `system` → Runtime
- Gradle: `implementation`/`api`/`runtimeOnly` → Runtime, `testImplementation`/`testRuntimeOnly` → Dev, `compileOnly`/`testCompileOnly` → Peer

## Risks / Trade-offs

**Gradle regex parsing is lossy** → Accept this. Dynamic builds that compute dependencies programmatically won't be captured. This is documented and matches our best-effort approach. Users can always override via `shire.toml` config.

**Maven parent POMs outside the repo** → If a parent POM is published to a remote repo, we can't resolve inherited values. Mitigation: child POMs that don't declare groupId/version and whose parent is external will fall back to directory-based naming.

**Gradle Kotlin DSL vs Groovy syntax** → The two have slightly different quoting and syntax. Mitigation: regex patterns handle both single and double quotes, and the `=` vs space assignment styles.

**`build` directory exclusion** → Adding `build` to default excludes could theoretically match a user's non-output directory named `build`. Mitigation: this is configurable via `shire.toml`.
