# Claude Code

Add Shire to your project's `.claude/settings.json`:

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

Or add globally in `~/.claude/settings.json` to use across all projects.

## Auto-rebuild hook

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
