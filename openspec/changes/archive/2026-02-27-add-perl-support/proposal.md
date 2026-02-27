## Why

Perl is a significant language in some monorepos (1,000+ `.pm` files) with no Shire support at all — no package indexing and no symbol extraction. Perl modules have clear structure: `package` declarations define namespaces, `sub` definitions define functions/methods, and dependencies are declared in `cpanfile`. Adding support would make this code searchable and navigable.

## What Changes

- Add a Perl package indexer that discovers packages from `cpanfile` manifest files
- Add a Perl symbol extractor using tree-sitter to extract `sub` definitions, `package` declarations, and method signatures from `.pm` and `.pl` files
- Register `cpanfile` as a manifest filename in the default discovery list
- Register `.pm` and `.pl` extensions and dispatchers in the symbol extraction system

Depends on `add-protobuf-support` for kind-agnostic symbol extraction — Perl symbols in directories already covered by another package type will be extracted automatically.

## Capabilities

### New Capabilities
- `perl-indexing`: Discover Perl packages from `cpanfile` manifests and extract symbols (subs, packages, methods) from `.pm`/`.pl` files

### Modified Capabilities
- `package-discovery`: Register Perl `cpanfile` manifest parser in the parser list
- `symbol-extraction`: Add `.pm`/`.pl` extension mappings and tree-sitter extractor dispatch
- `configuration`: Add `cpanfile` to default manifests list

## Impact

- **Code**: New `src/index/perl.rs` (cpanfile parser implementing `ManifestParser`), new `src/symbols/perl.rs` (symbol extractor), updates to `src/index/mod.rs` and `src/symbols/mod.rs` for registration
- **Dependencies**: `tree-sitter-perl` crate (new dependency in Cargo.toml)
- **DB**: New package and symbol rows; no schema changes
