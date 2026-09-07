# Setup

## Claude Code

One command configures shire globally for all projects:

```sh
shire init --global
```

This creates:
- `~/.claude/shire.toml` — shared config with `db_path = "~/.claude/shire/{repo}/{worktree}/index.db"` (auto-namespaced per repo and worktree)
- `mcpServers.shire` in `~/.claude.json` — serves the index via `shire serve`
- `PostToolUse` hook in `~/.claude/settings.json` — auto-rebuilds the index after file edits (`Edit`, `Write`, `NotebookEdit`, `Bash`)
- `~/.claude/rules/shire.md` — rules file guiding Claude Code to prefer Shire tools

The `{repo}` placeholder is replaced with the repository directory name at runtime, and `{worktree}` with the worktree name (or `_primary` for the main checkout), so each repo and worktree gets its own index file automatically.

After running `shire init --global`, open any repo and run:

```sh
shire build
```

The index is ready. Claude Code will automatically use it via the MCP server.

### Rules file

`shire init` creates `~/.claude/rules/shire.md` with guidance on when to use Shire tools vs Grep/Glob. This helps Claude Code default to Shire for codebase searches, so you spend fewer tool calls on broad exploration.

If it already exists, `shire init` leaves it untouched — with one exception: turning on the cross-reference index (`symbols.references_enabled`) for a repo that already has a rules file appends the extra reference-tools guidance in place, so your other customizations are preserved.

### CLAUDE.md integration

During interactive setup, `shire init` prompts:

> Add Shire search guidance to ~/.claude/CLAUDE.md?

If accepted, it appends a one-liner to `~/.claude/CLAUDE.md` directing Claude Code to prefer Shire MCP tools over Grep/Glob for code search. The line is idempotent — running init again won't duplicate it. If `~/.claude/CLAUDE.md` doesn't exist yet, it creates the file.

### Terminal output

`shire init` uses styled terminal output to show what it does:

- **✓** (green) — a file or config entry was created or updated
- **–** (dimmed) — a file or config entry already exists, skipped
- Section headers appear in **cyan**

Most file writes (`.gitignore`, `CLAUDE.md`, `settings.json`, `.mcp.json`, `~/.claude.json`) use **atomic writes** — content is written to a temporary file first, then renamed into place. This prevents partial writes if the process is interrupted.

### Project-level setup

To create a `shire.toml` in the current repo instead of globally:

```sh
shire init
```

This generates a minimal `shire.toml` (just `db_path`; everything else uses built-in defaults — see [Configuration](./configuration.md)), writes the MCP server config to `.mcp.json`, and (in hook mode) a `PostToolUse` hook to `.claude/settings.json` plus `.claude/rules/shire.md`. If the `db_path` points to a local directory (e.g., `.shire/index.db`), it offers to add that directory to `.gitignore`.

### Manual setup

If you prefer manual configuration, add to `~/.claude.json` (global) or `.mcp.json` (project-level):

```json
{
  "mcpServers": {
    "shire": {
      "command": "shire",
      "args": ["serve"]
    }
  }
}
```

To keep the index fresh during a session, add a `PostToolUse` hook to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write|NotebookEdit|Bash",
        "hooks": [{ "type": "command", "command": "shire rebuild --stdin" }]
      }
    ]
  }
}
```

## Claude Desktop

Add Shire to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "shire": {
      "command": "shire",
      "args": ["serve", "--db", "/path/to/repo/.shire/index.db"]
    }
  }
}
```

## Other MCP clients

Shire speaks standard MCP over stdio. Any client that supports MCP can connect:

```sh
shire serve --db /path/to/repo/.shire/index.db
```

Use `--root` to enable on-demand reindexing (before each query the server checks whether the index may be stale and rebuilds if so):

```sh
shire serve --root /path/to/repo
```

## Editor registration

For editors and CLIs other than Claude Code, `shire install` registers the built shire
binary as an MCP server with every supported tool it finds on the machine: Claude Code
(via the `claude` CLI, falling back to file patching), Codex CLI, Cursor, Windsurf,
Gemini CLI, VS Code, and Zed. It writes the current binary's absolute path (via
`std::env::current_exe`) rather than a bare `shire`, so registrations keep working even
if the tool that launches them doesn't inherit your shell's `PATH`.

```sh
shire install             # register with every detected tool
shire install --dry-run   # show what would change without writing anything
shire install --force     # overwrite existing registrations (e.g. after moving the binary)

shire uninstall           # remove shire's registration from every detected tool
shire uninstall --dry-run
```

`install`/`uninstall` only touch each tool's own MCP config file (or the tool's own CLI,
for Claude Code and Codex); they do not create or modify `shire.toml`, PostToolUse hooks,
or the rules file — use `shire init` for those.

## CLI reference

### Build an index

```sh
shire build --root /path/to/repo
```

### Rebuild from scratch

Ignore cached hashes and re-parse everything:

```sh
shire build --root /path/to/repo --force
```

### Custom database location

```sh
shire build --root /path/to/repo --db /tmp/my-index.db
```

The index defaults to `.shire/index.db` inside the repo root. Override with `--db` or `db_path` in `shire.toml` (see [Configuration](./configuration.md)).

### Clean up

Remove the index database, WAL/SHM files, the `.shire` directory, and stop the watch daemon:

```sh
shire clean
```

### Watch daemon status

Check whether the watch daemon is running for a repo (PID, socket path, and whether it's
actually reachable — see [Watch Daemon](./watch-daemon.md)):

```sh
shire watch --root /path/to/repo --status
```

## Incremental builds

Subsequent builds are **incremental** — only manifests whose content has changed (by SHA-256 hash) are re-parsed. Source files are tracked at **per-file granularity**: if individual source files change without a manifest change, only those files have their symbols re-extracted. An **mtime pre-check** skips hash computation entirely for packages whose source files haven't been touched since the last build.

File indexing is also incremental — a file-tree hash detects structural changes, skipping the file indexing phase entirely when no files have been added, removed, or resized.

Symbol extraction and source hashing are **parallelized** across packages and within packages using rayon for multi-core throughput. Files are read once per build (single-pass hash + extraction). All database writes use **batched multi-row INSERTs** within explicit transactions, with FTS5 triggers temporarily disabled during bulk operations for maximum SQLite throughput.

## Build progress

`shire build` shows real-time progress for each build phase:

- **Spinners** for quick phases (discovering manifests, workspace context, recomputing internals, indexing files)
- **Progress bars with ETAs** for longer phases (parsing manifests, extracting symbols)

Progress bars persist after completion so you can see the full build history in your terminal. Quiet mode (used internally by the MCP server for on-demand rebuilds) hides all progress output.
