## Why

The PostToolUse hook triggers `shire rebuild` after every Edit/Write/Bash tool call, causing redundant rebuilds (10 edits = 10 rebuild signals, even with debounce). The index only needs to be fresh when the MCP server answers a query. On-demand reindexing moves the rebuild to query time, eliminating wasted work and removing the need for the hook and watch daemon in most setups.

## What Changes

- MCP server gains the ability to rebuild the index on-demand before answering tool calls, with staleness detection via DB mtime
- MCP server sends MCP progress notifications during rebuilds so Claude Code shows rebuild status
- `shire serve` accepts `--root` and `--config` flags so it knows how to rebuild
- `shire init` supports `--no-hook` flag to skip PostToolUse hook installation (MCP server handles reindexing instead)
- `shire init --global` supports `--no-hook` similarly

## Capabilities

### New Capabilities

- `on-demand-reindex`: MCP server detects stale index and rebuilds before answering queries, with progress notifications over MCP protocol

### Modified Capabilities

- `mcp-server`: Server lifecycle changes — `shire serve` accepts `--root`/`--config`, can open DB read-write, and rebuilds on-demand before tool calls
- `configuration`: Config loading changes — `shire init` gains `--no-hook` flag that skips PostToolUse hook installation

## Impact

- `src/mcp/` — server gains rebuild capability, progress notifications, new CLI args
- `src/init.rs` — `--no-hook` flag for both local and global init
- `src/main.rs` — `serve` subcommand gains `--root` and `--config` args
- `src/index/mod.rs` — `build_index_quiet` needs to accept a progress callback or similar for MCP progress notifications
- README, CLAUDE.md — document new flag and on-demand mode
