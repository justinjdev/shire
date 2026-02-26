## Why

Java/Kotlin monorepos using Maven (`pom.xml`) and Gradle (`build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts`) are among the most common enterprise monorepo setups. Shire currently supports npm, Go, Cargo, and Python but has no JVM ecosystem coverage. Adding Maven and Gradle parsers closes this gap and makes shire useful for the large population of Java/Kotlin codebases.

## What Changes

- Add a Maven manifest parser (`pom.xml`) that extracts groupId:artifactId as the package name, version, description, and dependencies (compile, test, provided scopes)
- Add a Gradle manifest parser (`build.gradle` / `build.gradle.kts`) that extracts the project name, version, and dependencies from `dependencies {}` blocks using regex-based parsing (no Groovy/Kotlin eval)
- Add `settings.gradle` / `settings.gradle.kts` as workspace context files (like `go.work`) — parse `include` directives to identify multi-project builds and annotate members with workspace metadata
- Add `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts` to default manifest list in config
- Add `.gradle` to default exclude directories (build cache)
- Add `build` to default exclude directories (Gradle/Maven output)
- Register Java (`.java`) and Kotlin (`.kt`) source extensions for symbol extraction file walking — but actual tree-sitter extractors are **out of scope** for this change (symbols will be empty until extractors are added later)

## Capabilities

### New Capabilities
- `maven-parsing`: Parsing `pom.xml` files — artifact coordinates, parent POM inheritance context, dependency extraction with scopes
- `gradle-parsing`: Parsing `build.gradle`/`build.gradle.kts` files — project name/version extraction, dependency block parsing, `settings.gradle` workspace context

### Modified Capabilities
- `package-discovery`: Add `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts` to recognized manifest filenames; add `.gradle` and `build` to default excludes
- `configuration`: Add new manifest filenames and exclude dirs to defaults

## Impact

- **New files**: `src/index/maven.rs`, `src/index/gradle.rs`, `src/index/gradle_settings.rs`
- **Modified files**: `src/index/mod.rs` (register parsers, settings.gradle workspace context), `src/config.rs` (default manifests + excludes), `src/symbols/walker.rs` (java/kotlin extensions)
- **Dependencies**: No new crate dependencies — Maven uses existing `roxmltree` or simple XML parsing; Gradle uses regex
- **No breaking changes**: Existing ecosystems are unaffected
