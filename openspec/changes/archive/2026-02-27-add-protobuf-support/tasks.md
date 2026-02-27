## 1. Kind-agnostic symbol extraction

- [x] 1.1 Add `SymbolsConfig` struct to `src/config.rs` with `exclude_extensions: Vec<String>` field
- [x] 1.2 Add `[symbols]` section to `Config` struct and wire into `load_config`
- [x] 1.3 Add `all_extensions()` function to `src/symbols/walker.rs` returning all registered extensions
- [x] 1.4 Update `extract_symbols_for_package` in `src/symbols/mod.rs` to call `all_extensions()` instead of `extensions_for_kind(package_kind)`, filtering by `exclude_extensions` config
- [x] 1.5 Add tests for `all_extensions()` and exclude filtering
- [x] 1.6 Add test verifying a Gradle package now scans all registered extensions (not just `.java`/`.kt`)

## 2. Protobuf symbol extractor

- [x] 2.1 Add `tree-sitter-protobuf` dependency to `Cargo.toml`
- [x] 2.2 Create `src/symbols/proto.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 2.3 Implement message extraction (top-level and nested, kind: `Struct`)
- [x] 2.4 Implement service extraction (kind: `Interface`)
- [x] 2.5 Implement RPC extraction (kind: `Method`, with `parent_symbol`, parameters, return_type, streaming support)
- [x] 2.6 Implement enum extraction (top-level and nested, kind: `Enum`)
- [x] 2.7 Implement oneof extraction (kind: `Type`, with `parent_symbol`)

## 3. Registration and wiring

- [x] 3.1 Add `pub mod proto;` to `src/symbols/mod.rs`
- [x] 3.2 Add `"proto" => proto::extract(&source, &relative_path)` to the dispatch match block
- [x] 3.3 Add `.proto` to `all_extensions()` in `src/symbols/walker.rs`

## 4. Testing

- [x] 4.1 Add unit tests for proto extractor covering messages, services, RPCs, enums, oneofs, and nested types
- [x] 4.2 Add unit test for streaming RPC extraction
- [x] 4.3 Add unit test for unparseable proto file (extraction resilience)
- [x] 4.4 Add integration test: Gradle package directory with `.proto` files produces both Java and proto symbols
- [x] 4.5 Add config test: `exclude_extensions = [".proto"]` skips proto extraction
