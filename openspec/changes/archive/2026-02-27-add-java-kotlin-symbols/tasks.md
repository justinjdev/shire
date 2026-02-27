## 1. Java symbol extractor

- [x] 1.1 Add `tree-sitter-java` dependency to `Cargo.toml`
- [x] 1.2 Create `src/symbols/java.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 1.3 Implement class/interface/enum extraction (public and protected only)
- [x] 1.4 Implement method extraction (kind: `Method` with `parent_symbol`, parameters, return_type)
- [x] 1.5 Implement static method extraction (kind: `Function`)
- [x] 1.6 Implement constant extraction (`public static final` fields, kind: `Constant`)
- [x] 1.7 Implement visibility filtering (skip private and package-private)

## 2. Kotlin symbol extractor

- [x] 2.1 Add `tree-sitter-kotlin` dependency to `Cargo.toml`
- [x] 2.2 Create `src/symbols/kotlin.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 2.3 Implement class/interface/object/enum extraction
- [x] 2.4 Implement top-level function extraction (kind: `Function`)
- [x] 2.5 Implement class method extraction (kind: `Method` with `parent_symbol`)
- [x] 2.6 Implement visibility filtering (skip `private` and `internal`)

## 3. Registration and wiring

- [x] 3.1 Add `pub mod java;` and `pub mod kotlin;` to `src/symbols/mod.rs`
- [x] 3.2 Add `"java" => java::extract(...)` and `"kt" => kotlin::extract(...)` to dispatch match block
- [x] 3.3 Add `.java` and `.kt` to `all_extensions()` in `src/symbols/walker.rs` (if not already present from kind-agnostic change)
- [x] 3.4 Add `.gradle.kts` to `SKIP_SUFFIXES` in `src/symbols/walker.rs`

## 4. Testing

- [x] 4.1 Add unit tests for Java extractor: class, interface, enum, method, static method, constant, visibility filtering
- [x] 4.2 Add unit tests for Kotlin extractor: class, object, interface, enum, function, method, visibility filtering
- [x] 4.3 Add integration test: existing Gradle package gains Java/Kotlin symbols on rebuild
- [x] 4.4 Add test verifying `.gradle.kts` files are skipped
