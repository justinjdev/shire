## 1. npm workspace protocol

- [x] 1.1 In `npm.rs`, strip `workspace:` prefix from version strings when extracting deps
- [x] 1.2 Add test: package.json with `workspace:*`, `workspace:^`, `workspace:~1.0.0` deps — versions stored without prefix

## 2. Cargo workspace deps

- [x] 2.1 Add `collect_cargo_workspace_deps(path: &Path) -> Result<HashMap<String, String>>` function in `cargo.rs` that reads `[workspace.dependencies]` from a Cargo.toml
- [x] 2.2 Add `parse_with_workspace_deps()` method on `CargoParser` that accepts optional workspace deps map and resolves `workspace = true` entries
- [x] 2.3 Add test: member Cargo.toml with `dep = { workspace = true }` resolves version from workspace context
- [x] 2.4 Add test: `workspace = true` dep not in workspace map gets `version_req: None`

## 3. go.work parsing

- [x] 3.1 Create `src/index/go_work.rs` with `parse_go_work(path: &Path) -> Result<Vec<String>>` that extracts `use` directive paths
- [x] 3.2 Handle both single-line `use ./dir` and multi-line `use ( ... )` syntax
- [x] 3.3 Add test: parse go.work with multiple use directives
- [x] 3.4 Add `go.work` to `default_manifests()` in `config.rs`
- [x] 3.5 Add `go.work` to `default_manifests` test assertion count (4 → 5)

## 4. Orchestrator integration

- [x] 4.1 In `build_index`, after walk phase, scan Cargo.toml files for `[workspace]` sections and build workspace deps map
- [x] 4.2 In `build_index`, scan for `go.work` files and build set of workspace member directories
- [x] 4.3 During parse phase, call `parse_with_workspace_deps()` for Cargo members when workspace context exists
- [x] 4.4 During parse phase, set `metadata: {"go_workspace": true}` on Go packages whose directory matches a go.work `use` directive
- [x] 4.5 Ensure `go.work` files are walked but NOT parsed as packages (the GoWorkParser returns context, not a PackageInfo)
- [x] 4.6 Register `go_work` module in `src/index/mod.rs`

## 5. Integration tests

- [x] 5.1 Add integration test: Cargo workspace with root `[workspace.dependencies]` and member using `workspace = true` — verify resolved version in DB
- [x] 5.2 Add integration test: npm workspace with `workspace:*` dep — verify version stored as `*`
- [x] 5.3 Add integration test: go.work with use directive — verify Go package has `go_workspace` metadata
