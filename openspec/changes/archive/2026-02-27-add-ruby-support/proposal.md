## Why

Ruby support is already on the Shire roadmap. While the footprint may be small in some repos, Ruby has well-defined package conventions (`Gemfile`) and clear symbol structures (`class`, `module`, `def`). Adding support completes coverage for another mainstream language.

## What Changes

- Add a Ruby package indexer that discovers packages from `Gemfile` manifest files
- Add a Ruby symbol extractor using tree-sitter to extract `class`, `module`, `def` (methods/functions), and `attr_*` accessor definitions from `.rb` files
- Register `Gemfile` as a manifest filename in the default discovery list
- Register `.rb` extension and dispatcher in the symbol extraction system

Depends on `add-protobuf-support` for kind-agnostic symbol extraction — Ruby symbols in directories already covered by another package type will be extracted automatically.

## Capabilities

### New Capabilities
- `ruby-indexing`: Discover Ruby packages from `Gemfile` manifests and extract symbols (classes, modules, methods) from `.rb` files

### Modified Capabilities
- `package-discovery`: Register Ruby `Gemfile` manifest parser in the parser list
- `symbol-extraction`: Add `.rb` extension mapping and tree-sitter extractor dispatch
- `configuration`: Add `Gemfile` to default manifests list

## Impact

- **Code**: New `src/index/ruby.rs` (Gemfile parser implementing `ManifestParser`), new `src/symbols/ruby.rs` (symbol extractor), updates to `src/index/mod.rs` and `src/symbols/mod.rs` for registration
- **Dependencies**: `tree-sitter-ruby` crate (new dependency in Cargo.toml)
- **DB**: New package and symbol rows; no schema changes
