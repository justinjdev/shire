# Watch Daemon

`shire watch` starts a background daemon that auto-rebuilds the index when files change. It uses Unix domain socket IPC with configurable debounce (default 2s).

## Start the daemon

Idempotent — safe to call multiple times:

```sh
shire watch --root /path/to/repo
```

## Signal a rebuild manually

```sh
shire rebuild --root /path/to/repo
```

## Signal a rebuild from a Claude Code hook

Reads JSON from stdin, uses cwd as repo root:

```sh
shire rebuild --stdin
```

## Stop the daemon

```sh
shire watch --root /path/to/repo --stop
```

## Smart filtering

The watch daemon avoids unnecessary rebuilds:

- **Edit/Write tools** — checks file extension relevance and repo boundary
- **Bash commands** — filtered against a denylist of known read-only commands (`ls`, `git status`, `cargo test`, etc.) — unknown commands default to rebuild
