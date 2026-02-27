## Why

Shire indexes 1,424 Gradle packages but extracts zero symbols from the Java/Kotlin code behind them. The package structure is there but contains no searchable functions, classes, interfaces, or methods. Adding symbol extraction for Java and Kotlin would make the existing Gradle/Maven package indexing significantly more useful.

## What Changes

- Add a Java symbol extractor using tree-sitter to extract classes, interfaces, enums, methods, and fields from `.java` files
- Add a Kotlin symbol extractor using tree-sitter to extract classes, interfaces, objects, functions, and properties from `.kt` files
- Register `.java` and `.kt` extensions and dispatchers in the symbol extraction system

Depends on `add-protobuf-support` for kind-agnostic symbol extraction — once that lands, Java/Kotlin symbols automatically attach to existing Gradle/Maven packages without any package discovery changes.

## Capabilities

### New Capabilities
- `java-kotlin-symbols`: Extract symbols from Java (`.java`) and Kotlin (`.kt`) source files using tree-sitter, covering classes, interfaces, enums, methods, functions, and fields

### Modified Capabilities
- `symbol-extraction`: Add `.java` and `.kt` extension mappings and tree-sitter extractor dispatch

## Impact

- **Code**: New `src/symbols/java.rs`, new `src/symbols/kotlin.rs`, updates to `src/symbols/mod.rs` for dispatch registration
- **Dependencies**: `tree-sitter-java` and `tree-sitter-kotlin` crates (new dependencies in Cargo.toml)
- **DB**: New symbol rows for Java/Kotlin files; no schema changes
- **Existing packages**: All Maven and Gradle packages already in the index gain symbol data on next build
