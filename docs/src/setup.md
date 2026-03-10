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

The file is only written once — if it already exists, `shire init` leaves it untouched, so your customizations are preserved.

### Project-level setup

To create a `shire.toml` in the current repo instead of globally:

```sh
shire init
```

This generates a local config file with commented-out defaults you can customize, and writes the MCP server config to `.mcp.json`. If the `db_path` points to a local directory (e.g., `.shire/index.db`), it offers to add that directory to `.gitignore`.

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

Use `--root` to enable on-demand reindexing (the server checks `.git/index` mtime for staleness):

```sh
shire serve --root /path/to/repo
```

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

## Incremental builds

Subsequent builds are **incremental** — only manifests whose content has changed (by SHA-256 hash) are re-parsed. Source files are also tracked: if source files change without a manifest change, symbols are re-extracted automatically. An **mtime pre-check** skips SHA-256 computation entirely for packages whose source files haven't been touched since the last build.

File indexing is also incremental — a file-tree hash detects structural changes, skipping the file indexing phase entirely when no files have been added, removed, or resized.

Symbol extraction and source hashing are **parallelized** across packages using rayon for multi-core throughput. All database writes use **batched multi-row INSERTs** within explicit transactions for maximum SQLite throughput.
