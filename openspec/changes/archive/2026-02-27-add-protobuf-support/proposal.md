## Why

Protobuf files define API contracts and data schemas across services, making them critical for understanding cross-service dependencies in a monorepo. Shire currently has no support for extracting symbols from `.proto` files. Proto files aren't packages — they're cross-cutting definitions that live within existing packages. Their symbols (messages, services, RPCs, enums) should be searchable wherever they appear.

## What Changes

- Add a Protobuf symbol extractor using tree-sitter to extract `message`, `service`, `rpc`, `enum`, and `oneof` definitions from `.proto` files
- Make symbol extraction kind-agnostic: extract symbols from ALL registered file extensions within a package, not just the owning package's kind. A Gradle package with `.proto` files gets both Java and proto symbols.
- Add a `[symbols]` config section with an `exclude_extensions` list so users can skip symbol extraction for file types they don't care about (e.g., `exclude_extensions = [".proto"]` to opt out)
- Register `.proto` in the default set of symbol-extractable extensions

Example config:
```toml
[symbols]
exclude_extensions = [".proto"]  # skip proto symbol extraction
```

## Capabilities

### New Capabilities
- `protobuf-symbols`: Tree-sitter symbol extractor for `.proto` files covering messages, services, RPCs, enums, and oneofs

### Modified Capabilities
- `symbol-extraction`: Make extraction kind-agnostic (run all registered extractors based on file extension, not package kind). Add `.proto` extension and dispatcher entry.
- `configuration`: Add `[symbols].exclude_extensions` list for opting out of symbol extraction per file type

## Impact

- **Code**: New `src/symbols/proto.rs` (symbol extractor), changes to `src/symbols/mod.rs` (kind-agnostic dispatch), `src/symbols/walker.rs` (extension registration), `src/config.rs` (symbols config)
- **Dependencies**: `tree-sitter-protobuf` crate (new dependency in Cargo.toml)
- **Behavior change**: Symbol extraction becomes kind-agnostic — existing packages may gain symbols from file types that were previously ignored. This is additive (more symbols, no symbols lost). Users can exclude unwanted extensions via config.
- **DB**: New symbol rows for proto files; no schema changes
