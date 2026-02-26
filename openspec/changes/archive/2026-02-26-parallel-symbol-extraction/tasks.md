## 1. Add rayon dependency

- [x] 1.1 Add `rayon = "1"` to `[dependencies]` in `Cargo.toml`

## 2. Parallelize Phase 7

- [x] 2.1 Refactor `phase_extract_symbols` to use `par_iter` over `parsed_packages`, collecting `(name, Result<symbols>, Result<hash>)` tuples
- [x] 2.2 Sequential DB insert loop after parallel collection: `upsert_symbols` + `upsert_source_hash` for each result

## 3. Parallelize Phase 8

- [x] 3.1 Refactor `phase_source_incremental` to pre-fetch package info and stored hashes from DB into a Vec
- [x] 3.2 Use `par_iter` over pre-fetched packages to compute current hashes and conditionally extract symbols
- [x] 3.3 Sequential DB insert loop for re-extracted packages, return correct `num_reextracted` count

## 4. Verify

- [x] 4.1 Run `cargo build` — clean compile
- [x] 4.2 Run `cargo test` — 143 tests pass (120 unit + 23 integration)
