## 1. Serve command --root flag

- [x] 1.1 Add `--root` optional flag to `Serve` subcommand in `src/main.rs`
- [x] 1.2 When `--root` is provided, pass `repo_root`, `config`, and `db_path` to `ShireService::new()`
- [x] 1.3 When `--root` is provided and DB does not exist, start server normally (no bail) — first tool call triggers build

## 2. ShireService build context

- [x] 2.1 Add `repo_root: Option<PathBuf>`, `config: Option<Config>`, `db_path: PathBuf` fields to `ShireService`
- [x] 2.2 Update `ShireService::new()` to accept and store the new fields
- [x] 2.3 Update `run_server()` signature to accept optional build context and pass through to `ShireService::new()`

## 3. Staleness detection

- [x] 3.1 Add `last_indexed: Mutex<Option<SystemTime>>` field to `ShireService`, initialized from `shire_meta.indexed_at` (or `None` if no DB)
- [x] 3.2 Implement `is_stale()` method: stat `.git/index` mtime, compare against `last_indexed`, return `bool`
- [x] 3.3 Handle edge cases: no `.git/index` (return false), no existing DB (return true)

## 4. On-demand rebuild

- [x] 4.1 Implement `maybe_rebuild(&self)` method on `ShireService` — calls `is_stale()`, runs `build_index_quiet`, reopens connection, updates `last_indexed`
- [x] 4.2 On rebuild failure, log error via `eprintln!` and continue with existing index (do not crash)
- [x] 4.3 Add `self.maybe_rebuild()` call at the top of each `#[tool]` handler

## 5. MCP logging notifications

- [x] 5.1 ~~Store `Peer<RoleServer>`~~ — simplified to `eprintln!` stderr logging (MCP peer notifications deferred)
- [x] 5.2 Send `Info` logging notification before rebuild: "Rebuilding index..." (via eprintln)
- [x] 5.3 Send `Info` logging notification after rebuild: "Index rebuilt" (via eprintln)

## 6. Init --no-hook flag

- [x] 6.1 Add `--no-hook` flag to `Init` command in `src/main.rs`
- [x] 6.2 Pass `no_hook: bool` through to `run_init()` and `run_init_global()`
- [x] 6.3 When `no_hook` is true, call `patch_claude_settings` with `serve_args: json!(["serve", "--root", "."])` and skip hook installation
- [x] 6.4 Extract hook installation into a conditional block in `patch_claude_settings` (add `install_hook: bool` parameter)
- [x] 6.5 Print appropriate next-step instructions for no-hook mode

## 7. Tests

- [x] 7.1 Unit test: `is_stale()` returns false when `.git/index` is older than `last_indexed`
- [x] 7.2 Unit test: `is_stale()` returns true when `.git/index` is newer than `last_indexed`
- [x] 7.3 Unit test: `is_stale()` returns false when no `.git/index` exists
- [x] 7.4 Unit test: `maybe_rebuild()` is no-op when `repo_root` is `None` (read-only mode)
- [x] 7.5 Unit test: `shire init --no-hook` creates MCP config with `["serve", "--root", "."]` and no PostToolUse hook
- [x] 7.6 Unit test: `shire init --no-hook` idempotent (running twice does not duplicate entries)
- [x] 7.7 Unit test: `shire init` (without --no-hook) still installs PostToolUse hook as before

## 8. Documentation

- [x] 8.1 Update README: document `--no-hook` flag and on-demand mode in setup section
- [x] 8.2 Update README: add `--root` to serve command reference table
- [x] 8.3 Update CLAUDE.md: note on-demand reindexing option
