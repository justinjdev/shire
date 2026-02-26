## 1. Dependencies and types

- [x] 1.1 Add tree-sitter dependencies to Cargo.toml: `tree-sitter`, `tree-sitter-typescript`, `tree-sitter-javascript`, `tree-sitter-go`, `tree-sitter-rust`, `tree-sitter-python`
- [x] 1.2 Create `src/symbols/mod.rs` with `SymbolInfo`, `SymbolKind` enum, `Parameter` struct, and `extract_symbols_for_package()` orchestrator function signature
- [x] 1.3 Create `src/symbols/walker.rs` — walk a package directory for source files by extension, skipping excluded directories
- [x] 1.4 Add tests for walker: correct extensions per ecosystem, excluded dirs skipped

## 2. DB schema and queries

- [x] 2.1 Add `symbols` table, indexes, `symbols_fts` virtual table, and FTS triggers to `src/db/mod.rs`
- [x] 2.2 Add `SymbolRow` struct to `src/db/queries.rs`
- [x] 2.3 Add `search_symbols(conn, query, package_filter, kind_filter) -> Vec<SymbolRow>` with FTS5
- [x] 2.4 Add `get_package_symbols(conn, package, kind_filter) -> Vec<SymbolRow>`
- [x] 2.5 Add `get_symbol(conn, name, package_filter) -> Vec<SymbolRow>`
- [x] 2.6 Add query tests: search by name, search by signature, filter by package, filter by kind, empty query

## 3. TypeScript/JavaScript extractor

- [x] 3.1 Create `src/symbols/typescript.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 3.2 Extract exported functions with full signature, parameters, return type
- [x] 3.3 Extract exported classes, and public methods within them (kind=method, parent_symbol set)
- [x] 3.4 Extract exported interfaces, type aliases, enums, constants
- [x] 3.5 Handle default exports
- [x] 3.6 Skip non-exported symbols
- [x] 3.7 Add tests: function with params + return type, class with methods, interface, type alias, enum, const, default export, non-exported skipped

## 4. Go extractor

- [x] 4.1 Create `src/symbols/go.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 4.2 Extract exported functions (uppercase first letter) with full signature, parameters, return type
- [x] 4.3 Extract exported type declarations (struct, interface)
- [x] 4.4 Extract methods with receiver — kind=method, parent_symbol set to receiver type
- [x] 4.5 Skip unexported symbols (lowercase first letter)
- [x] 4.6 Add tests: exported function, struct, interface, method with receiver, unexported skipped

## 5. Rust extractor

- [x] 5.1 Create `src/symbols/rust_lang.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 5.2 Extract pub functions with full signature, parameters, return type
- [x] 5.3 Extract pub structs, enums, traits
- [x] 5.4 Extract pub methods inside impl blocks — kind=method, parent_symbol set to impl target
- [x] 5.5 Skip non-pub symbols
- [x] 5.6 Add tests: pub function, pub struct, pub enum, pub trait, impl method, non-pub skipped

## 6. Python extractor

- [x] 6.1 Create `src/symbols/python.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 6.2 Extract top-level function definitions with parameters and return type annotations
- [x] 6.3 Extract class definitions, and public methods within them (skip `_` prefix except `__init__`)
- [x] 6.4 Add tests: function with type hints, class with methods, __init__ extracted, _private skipped

## 7. Build pipeline integration

- [x] 7.1 Register `symbols` module in `src/main.rs` or `src/lib.rs`
- [x] 7.2 Implement `extract_symbols_for_package()` in `src/symbols/mod.rs` — dispatch to correct extractor based on package kind
- [x] 7.3 Add `upsert_symbols()` function — clear old symbols for package, insert new ones
- [x] 7.4 Add phase 7 to `build_index` in `src/index/mod.rs` — after parsing, extract symbols for each new/changed package
- [x] 7.5 Ensure `--force` clears symbols table
- [x] 7.6 Ensure deleted packages have their symbols removed
- [x] 7.7 Add symbol count to build summary output

## 8. MCP tools

- [x] 8.1 Add `SearchSymbolsParams` struct with query, optional package, optional kind
- [x] 8.2 Add `GetPackageSymbolsParams` struct with package name, optional kind
- [x] 8.3 Add `GetSymbolParams` struct with name, optional package
- [x] 8.4 Implement `search_symbols` tool handler
- [x] 8.5 Implement `get_package_symbols` tool handler
- [x] 8.6 Implement `get_symbol` tool handler

## 9. Integration tests

- [x] 9.1 Add integration test: build index on fixture with TS file containing exported function — verify symbol in DB with correct signature, params, return type
- [x] 9.2 Add integration test: build index on fixture with Go file containing exported struct + method — verify symbols with parent_symbol
- [x] 9.3 Add integration test: build index on fixture with Rust file containing pub fn — verify symbol in DB
- [x] 9.4 Add integration test: incremental rebuild after source change — verify symbols updated
- [x] 9.5 Add integration test: search_symbols FTS query returns matching symbols
