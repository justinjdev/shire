# Claude Code

## Quick setup

One command configures shire globally for all projects:

```sh
shire init --global
```

This creates:
- `~/.claude/shire.toml` — shared config with `db_path = "~/.claude/shire/{repo}/index.db"` (auto-namespaced per repo)
- `mcpServers.shire` in `~/.claude/settings.json` — serves the index via `shire serve --config ~/.claude/shire.toml`
- `PostToolUse` hook — auto-rebuilds the index after file edits (`Edit`, `Write`, `NotebookEdit`, `Bash`)

The `{repo}` placeholder is replaced with the repository directory name at runtime, so each repo gets its own index file automatically.

After running `shire init --global`, open any repo and run:

```sh
shire build
```

The index is ready. Claude Code will automatically use it via the MCP server.

## Project-level setup

To create a `shire.toml` in the current repo instead of globally:

```sh
shire init
```

This generates a config file with commented-out defaults you can customize.

## Manual setup

If you prefer manual configuration, add to `~/.claude/settings.json` (or project-level `.claude/settings.json`):

```json
{
  "mcpServers": {
    "shire": {
      "command": "shire",
      "args": ["serve", "--config", "~/.claude/shire.toml"]
    }
  }
}
```

### Auto-rebuild hook

To keep the index fresh during a session, add a `PostToolUse` hook that signals the watch daemon after file-modifying tools:

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
