## 1. Gemfile manifest parser

- [x] 1.1 Create `src/index/ruby.rs` with `RubyParser` struct implementing `ManifestParser`
- [x] 1.2 Implement `filename()` returning `"Gemfile"`
- [x] 1.3 Implement `parse()`: extract `gem 'name', 'version'` lines as Runtime deps
- [x] 1.4 Handle `group :test` and `group :development` blocks as Dev deps
- [x] 1.5 Handle unversioned dependencies (`gem 'name'`)
- [x] 1.6 Handle unparseable lines (silently skip)
- [x] 1.7 Set package name to directory path relative to repo root, kind to `"ruby"`

## 2. Ruby symbol extractor

- [x] 2.1 Add `tree-sitter-ruby` dependency to `Cargo.toml`
- [x] 2.2 Create `src/symbols/ruby.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 2.3 Implement class extraction (kind: `Class`, including inheritance in signature)
- [x] 2.4 Implement module extraction (kind: `Class`)
- [x] 2.5 Implement instance method extraction (kind: `Method` with `parent_symbol`, parameters)
- [x] 2.6 Implement class method `def self.name` extraction (kind: `Function`)
- [x] 2.7 Implement top-level method extraction (kind: `Function`)

## 3. Registration and wiring

- [x] 3.1 Add `pub mod ruby;` to `src/index/mod.rs` and register `RubyParser` in parser list
- [x] 3.2 Add `pub mod ruby;` to `src/symbols/mod.rs`
- [x] 3.3 Add `"rb" => ruby::extract(...)` to dispatch match block
- [x] 3.4 Add `.rb` to `all_extensions()` in `src/symbols/walker.rs`
- [x] 3.5 Add `"Gemfile"` to `default_manifests()` in `src/config.rs`

## 4. Testing

- [x] 4.1 Add unit tests for Gemfile parser: simple deps, versioned deps, group deps, unparseable lines
- [x] 4.2 Add unit tests for Ruby symbol extractor: class, module, instance method, class method, top-level method, inheritance
- [x] 4.3 Add integration test: directory with Gemfile and `.rb` files produces package with Ruby symbols
