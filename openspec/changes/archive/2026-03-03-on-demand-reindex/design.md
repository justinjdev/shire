## Context

Currently `shire serve` opens the SQLite index read-only and never rebuilds. Freshness is maintained externally via a PostToolUse hook that calls `shire rebuild` after every file edit, which signals the watch daemon to run `build_index`. This works but is wasteful — most rebuilds are redundant because no MCP query follows immediately.

The MCP server runs over stdio, so it cannot use indicatif progress bars (that would corrupt the protocol stream). However, `rmcp` exposes `Peer<RoleServer>::notify_progress()` which sends MCP `notifications/progress` — Claude Code renders these natively.

`shire serve` already accepts `--config` and resolves `db_path` from config. It uses `canonicalize(".")` as repo root. The `ShireService` struct holds a `Mutex<Connection>` shared across all tool handlers via `#[tool_router]`.

## Goals / Non-Goals

**Goals:**
- MCP server detects stale index and rebuilds before answering tool calls
- Rebuild progress is visible in Claude Code via MCP progress notifications
- `shire init --no-hook` provides a clean setup without PostToolUse hooks
- Zero-config experience: on-demand mode works without watch daemon or hooks

**Non-Goals:**
- Replacing the hook-based mode — both modes coexist, user chooses at init time
- Per-file incremental rebuild within the MCP server — full `build_index_quiet` is sufficient since it's already incremental via content hashing
- Granular per-phase progress over MCP — a single "Rebuilding index..." → "Done" notification is sufficient

## Decisions

### 1. Staleness detection: DB file mtime

Compare the DB file's mtime against a `last_checked` timestamp stored in memory on the `ShireService`. On each tool call, stat the DB path — if mtime is newer than `last_checked`, no rebuild needed (something else already rebuilt). If no DB exists or mtime is older than a threshold, trigger a rebuild.

For source staleness (has code changed since last build?), use a lightweight check: stat the repo root's `.git/index` mtime (reflects any staged/committed changes) and compare against the `indexed_at` value in `shire_meta`. This avoids walking the entire source tree on every tool call.

**Alternative considered:** Tracking a dirty flag in a file or the DB. Rejected because it requires an external writer (the hook) to set the flag, which defeats the purpose of eliminating hooks.

**Alternative considered:** Walking source files for mtime checks. Rejected as too expensive to run on every MCP tool call. The `.git/index` heuristic catches most changes; the incremental build handles the rest when triggered.

### 2. Rebuild trigger: before tool call, inside `call_tool`

Add a `maybe_rebuild` method on `ShireService` that checks staleness and rebuilds if needed. Each `#[tool]` handler calls `self.maybe_rebuild()` at the top. This is simpler than trying to intercept at the rmcp framework level.

The rebuild calls `build_index_quiet` (no indicatif) and then reopens the connection by replacing the `Mutex<Connection>` contents. The DB is opened read-write for the rebuild, then reopened read-only for queries.

**Alternative considered:** Middleware/interceptor pattern in rmcp. The `#[tool_handler]` macro doesn't expose a clean hook point, and wrapping every handler is more complex than a one-line `self.maybe_rebuild()` call.

### 3. Progress notifications: coarse-grained

Send two MCP progress notifications per rebuild:
1. `progress: 0, total: 1, message: "Rebuilding index…"` — before rebuild
2. `progress: 1, total: 1, message: "Index rebuilt"` — after rebuild

This requires passing a `ProgressToken` into the rebuild path. Since `maybe_rebuild` runs synchronously (tool handlers are sync in the current codebase), and `notify_progress` is async, we'll use a stored `tokio::runtime::Handle` to block on the notification send.

Progress tokens come from the client via `context.meta.get_progress_token()`. Since `maybe_rebuild` is called from within tool handlers, the token needs to be passed through or stored temporarily.

**Simplification:** Rather than threading progress tokens through `maybe_rebuild`, use MCP logging notifications (`notify_logging_message`) instead — these don't require a token and are always visible. Send an `Info` log before and after rebuild.

### 4. ShireService gains build context

`ShireService::new()` accepts additional fields:
- `repo_root: PathBuf` — needed for `build_index_quiet`
- `config: Config` — needed for `build_index_quiet`
- `db_path: PathBuf` — needed to reopen connection after rebuild

These are stored on the struct. When on-demand mode is not configured (no repo_root provided), `maybe_rebuild` is a no-op.

### 5. `shire serve` gains `--root` flag

The `Serve` subcommand gets an optional `--root` flag (default: `.`). When provided, the server operates in on-demand mode. When omitted, behavior is unchanged (read-only, no rebuilds).

`shire init --no-hook` wires `args: ["serve", "--root", "."]` in the MCP config instead of `args: ["serve"]`, and skips the PostToolUse hook.

### 6. `--no-hook` flag on init

Both `shire init` and `shire init --global` accept `--no-hook`. When set:
- Skip PostToolUse hook installation entirely
- Add `--root` to the MCP server args so it enables on-demand mode
- Print instructions reflecting the no-hook setup

The flag is purely an init-time concern — it controls what gets written to settings.json. No config file (`shire.toml`) changes needed.

## Risks / Trade-offs

**First-query latency** — The first MCP tool call after edits triggers a synchronous rebuild. For large repos this could be 1-2 seconds. Mitigation: the incremental build with mtime pre-checks is fast for small changes (typically <500ms).

**Concurrent tool calls during rebuild** — If Claude Code sends multiple tool calls while a rebuild is in progress, they'll block on the `Mutex<Connection>`. This is acceptable since MCP tool calls are typically sequential. Mitigation: use a `RwLock` or rebuild flag to let read queries proceed if no rebuild is needed.

**`.git/index` heuristic misses** — Changes made outside git (e.g., editor autosave without staging) won't update `.git/index` mtime. Mitigation: the mtime check is a heuristic fast-path; if it misses, the next `shire build` or a manual refresh catches it. For most Claude Code usage, edits go through Edit/Write tools which commit to disk immediately.

**DB reopening** — Replacing the connection inside a `Mutex` while other tool calls might be waiting is safe because the mutex serializes access. The old connection is dropped when replaced.
