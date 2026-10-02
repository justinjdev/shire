# shire-status: a Claude Code mod

Shows shire's index health inside Claude Code. It reads `shire status --json`,
which never rebuilds or writes.

- **Status line**: `shire ● 412 pkgs · 38.2k syms · 4m ago`, with `⟳` while a
  build runs, `▲` for warnings (build failures, an interrupted build, a capped
  or partial file walk, pending re-checks, `HEAD moved` since the index was
  built) and `✗` when the index is missing or unreadable. Polled every 15 s and
  3 s after any edit.
- **Toasts** only when something changed: symbol/file counts moved, new build
  failures, an interrupted build, the 500k file cap, the watch daemon stopping.
  An on-demand rebuild (`serve --root`) that changed nothing stays quiet.
- **`/shire`** opens a pane with the details and **Rebuild** / **Force rebuild**
  buttons. Rebuild goes through the watch daemon when it is running, otherwise
  runs `shire build`. `/shire rebuild [--force]` does the same from the prompt.

## Requirements

A `shire` on `PATH` that has the `status` subcommand.

## Install

Load the folder as a plugin directory:

```sh
claude --plugin-dir /path/to/shire/contrib/claude-mod/shire-status
```

or, where no flag can be passed (desktop app, SDK), list it in
`CLAUDE_CODE_PLUGIN_DIRS` (process environment or the `env` block of
`~/.claude/settings.json`).

## Develop

```sh
claude plugin validate contrib/claude-mod/shire-status
claude plugin test contrib/claude-mod/shire-status
```

Claude Code lays the API's type declarations into `.claude-plugin/types/` when
it loads the folder (git-ignored); after that `tsc -p contrib/claude-mod/shire-status`
type-checks it. The mod API is early access and may change between Claude Code
releases.
