## 1. Config

- [x] 1.1 Add `CustomDiscoveryRule` struct to `src/config.rs` with fields: `name`, `kind`, `requires`, `paths`, `exclude`, `max_depth`, `name_prefix`, `extensions`
- [x] 1.2 Add `custom: Vec<CustomDiscoveryRule>` to `DiscoveryConfig`
- [x] 1.3 Add validation: error if `name`, `kind`, or `requires` are missing
- [x] 1.4 Add config tests: valid rule parsing, missing required fields, empty custom list

## 2. Discovery engine

- [x] 2.1 Create discovery function in `src/index/mod.rs` (or new `src/index/custom_discovery.rs`) that takes `&[CustomDiscoveryRule]` and repo root, returns `Vec<PackageInfo>`
- [x] 2.2 Implement directory walking with `paths` scoping (default to repo root)
- [x] 2.3 Implement `max_depth` limiting relative to each `paths` entry
- [x] 2.4 Implement rule-specific `exclude` merged with global excludes
- [x] 2.5 Implement glob matching for `requires` patterns against directory contents
- [x] 2.6 Implement nested match prevention (skip subdirectories of matched directories)
- [x] 2.7 Implement package naming: `{name_prefix}{relative_path}`

## 3. Pipeline integration

- [x] 3.1 Wire custom discovery into build pipeline after manifest walk phase
- [x] 3.2 Merge custom-discovered packages into the parsed packages list (dedup by path, custom wins)
- [x] 3.3 Ensure custom packages get symbol extraction (respect `extensions` override if set)

## 4. Testing

- [x] 4.1 Add unit tests: rule matching with exact filenames, glob patterns, missing files
- [x] 4.2 Add unit tests: path scoping, max_depth, rule-specific excludes
- [x] 4.3 Add unit test: nested match prevention
- [x] 4.4 Add unit test: name_prefix applied correctly
- [x] 4.5 Add unit test: deduplication with manifest-discovered packages
- [x] 4.6 Add integration test: custom rule discovers Go apps with `main.go` + `ownership.yml`
- [x] 4.7 Add config test: no `[[discovery.custom]]` rules = no custom discovery runs
