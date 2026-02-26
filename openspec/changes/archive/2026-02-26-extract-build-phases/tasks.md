## 1. Introduce new types

- [x] 1.1 Add `WorkspaceContext` struct (replaces `BuildContext` — groups workspace-specific context instead of conn/root/config)
- [x] 1.2 Add `BuildSummary` struct with counters for added, changed, removed, skipped, source_reextracted, symbols, files, and failures vec

## 2. Extract phase functions

- [x] 2.1 Extract Phase 1+1.5 (walk + workspace context) into `phase_walk_and_context` returning `(Vec<WalkedManifest>, WorkspaceContext)`
- [x] 2.2 Phase 2 (diff) already factored as `diff_manifests` — kept inline in orchestrator
- [x] 2.3 Extract Phase 3 (parse changed manifests) into `phase_parse` returning parsed packages and failures
- [x] 2.4 Extract Phase 4 (remove deleted) into `phase_remove_deleted`
- [x] 2.5 Phase 5 (recompute is_internal) already a function — kept as conditional call
- [x] 2.6 Extract Phase 6 (store manifest hashes) into `phase_store_hashes`
- [x] 2.7 Extract Phase 7 (extract symbols for new/changed) into `phase_extract_symbols`
- [x] 2.8 Extract Phase 8 (source-level incremental) into `phase_source_incremental` returning count
- [x] 2.9 Extract Phase 9 (file indexing) into `phase_index_files` returning file count

## 3. Refactor build_index

- [x] 3.1 Rewrite `build_index` as thin orchestrator calling phase functions and assembling `BuildSummary`
- [x] 3.2 Move summary printing and metadata storage into `print_summary` and `store_metadata` consuming `BuildSummary`

## 4. Verify

- [x] 4.1 Run `cargo test` — 143 tests pass (120 unit + 23 integration)
- [x] 4.2 Run `cargo build` — clean compile, no new warnings
