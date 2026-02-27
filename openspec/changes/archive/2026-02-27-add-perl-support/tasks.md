## 1. cpanfile manifest parser

- [x] 1.1 Create `src/index/perl.rs` with `PerlParser` struct implementing `ManifestParser`
- [x] 1.2 Implement `filename()` returning `"cpanfile"`
- [x] 1.3 Implement `parse()`: extract `requires 'Name', 'version';` lines as Runtime deps
- [x] 1.4 Handle `on 'test'` blocks as Dev deps
- [x] 1.5 Handle unversioned dependencies (`requires 'Name';`)
- [x] 1.6 Handle unparseable lines (silently skip)
- [x] 1.7 Set package name to directory path relative to repo root, kind to `"perl"`

## 2. Perl symbol extractor

- [x] 2.1 ~~Add `tree-sitter-perl` dependency~~ Used regex-based extraction (tree-sitter-perl requires ts 0.26, incompatible with 0.24)
- [x] 2.2 Create `src/symbols/perl.rs` with `extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`
- [x] 2.3 Implement `package` declaration extraction (kind: `Class`)
- [x] 2.4 Implement `sub` extraction — top-level as `Function`, inside package as `Method` with `parent_symbol`
- [x] 2.5 Implement visibility filtering (skip subs prefixed with `_`)

## 3. Registration and wiring

- [x] 3.1 Add `pub mod perl;` to `src/index/mod.rs` and register `PerlParser` in parser list
- [x] 3.2 Add `pub mod perl;` to `src/symbols/mod.rs`
- [x] 3.3 Add `"pm" | "pl" => perl::extract(...)` to dispatch match block
- [x] 3.4 Add `.pm` and `.pl` to `all_extensions()` in `src/symbols/walker.rs`
- [x] 3.5 Add `"cpanfile"` to `default_manifests()` in `src/config.rs`

## 4. Testing

- [x] 4.1 Add unit tests for cpanfile parser: simple deps, versioned deps, test deps, unparseable lines
- [x] 4.2 Add unit tests for Perl symbol extractor: package declarations, top-level subs, methods in packages, private sub filtering
- [x] 4.3 Add integration test: directory with cpanfile and `.pm` files produces package with Perl symbols
