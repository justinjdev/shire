## Context

The walker already maps `maven`/`gradle` kinds to `["java", "kt"]` extensions (`walker.rs:35`), but no extractors exist for these extensions — the dispatch in `extract_symbols_for_package` falls through to `_ => Vec::new()`. With kind-agnostic extraction from `add-protobuf-support`, adding Java/Kotlin extractors is purely additive: implement the extract functions, register them in the dispatch match block.

Existing extractors (TypeScript, Go, Rust, Python) follow a consistent pattern: parse with tree-sitter, walk the AST for top-level declarations, extract name/kind/signature/parameters/return_type, filter by visibility.

## Goals / Non-Goals

**Goals:**
- Extract symbols from `.java` files (classes, interfaces, enums, methods, fields)
- Extract symbols from `.kt` files (classes, interfaces, objects, functions, properties)
- Follow existing extractor patterns for consistency

**Non-Goals:**
- Java/Kotlin package discovery (already handled by Maven/Gradle parsers)
- Annotation extraction or processing
- Inner class / anonymous class extraction (top-level and named member classes only)

## Decisions

### 1. Separate extractors per language

Java and Kotlin have different ASTs despite JVM similarities. Two separate files (`java.rs`, `kotlin.rs`) with their own tree-sitter parsers, following the existing pattern of one file per language.

### 2. Visibility filtering

**Java:** Extract only `public` and `protected` declarations. Skip `private` and package-private. This matches the Go/Rust convention of extracting the public API surface.

**Kotlin:** Extract all declarations without `private` or `internal` visibility modifiers. Kotlin defaults to `public`.

### 3. Symbol kind mapping

| Java construct | SymbolKind |
|---|---|
| `class` | `Class` |
| `interface` | `Interface` |
| `enum` | `Enum` |
| `method` | `Method` (with `parent_symbol`) |
| `static method` | `Function` |
| `constant (static final)` | `Constant` |

| Kotlin construct | SymbolKind |
|---|---|
| `class` | `Class` |
| `interface` | `Interface` |
| `object` | `Class` |
| `enum class` | `Enum` |
| `fun` (top-level) | `Function` |
| `fun` (in class) | `Method` (with `parent_symbol`) |
| `val`/`var` (top-level const) | `Constant` |

### 4. tree-sitter crates

`tree-sitter-java` and `tree-sitter-kotlin`. Both are mature, well-maintained grammars.

## Risks / Trade-offs

**[Volume] Java/Kotlin codebases can be massive** → Mitigation: Extraction is already parallelized via rayon. Visibility filtering reduces symbol count. Users can exclude `.java`/`.kt` via `[symbols].exclude_extensions`.

**[Kotlin DSL files] `build.gradle.kts` files are Kotlin** → Mitigation: These are already filtered by `SKIP_SUFFIXES` or excluded directories. If not, they'd produce build-script symbols which is noise. May need to add `.gradle.kts` to skip patterns.
