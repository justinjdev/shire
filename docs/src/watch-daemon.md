# Watch Daemon

`shire watch` starts a background daemon that rebuilds the index whenever a rebuild
*signal* arrives — from the Claude Code `PostToolUse` hook (`shire rebuild --stdin`) or a
manual `shire rebuild`. **It does not watch the filesystem itself** (no inotify/FSEvents):
an edit made outside Claude Code — by another editor, `git checkout`, a codegen script run
outside the hook — is never picked up until something explicitly signals a rebuild. It
uses Unix domain socket IPC with configurable debounce (default 2s).

## Start the daemon

Idempotent — safe to call multiple times:

```sh
shire watch --root /path/to/repo
```

## Check whether it's running

```sh
shire watch --root /path/to/repo --status
```

Prints the daemon's PID, socket path, and whether it's actually reachable (a stale PID
file left behind by a crash, `kill -9`, or a bind failure reads as not running, not as a
false "yes").

## Signal a rebuild manually

```sh
shire rebuild --root /path/to/repo
```

If no daemon is listening at that root, this prints a warning to stderr and still exits
0 (so it's safe to call from a hook) — the index is simply not updated.

## Signal a rebuild from a Claude Code hook

Reads JSON from stdin. The repo root is resolved by walking up from the hook's `cwd` to
the nearest ancestor containing `shire.toml` or `.git` — so a session launched in a
package subdirectory of a monorepo still reaches the daemon's socket at the repo root. A
bare `.shire/` directory does **not** count as a marker: shire creates `.shire/logs`
under any directory it is pointed at, so treating it as a marker would let a stray
`.shire` left behind by a one-off `shire build --root <subdir>` silently divert future
lookups to that subdirectory instead of the real repo root:

```sh
shire rebuild --stdin
```

## Stop the daemon

```sh
shire watch --root /path/to/repo --stop
```

Sends SIGTERM and waits up to 5s for the process to actually exit before removing its
PID/socket files. The daemon only handles SIGTERM between rebuilds, so a slow or
uninterruptible rebuild can outlast that wait — if it does, `--stop` exits non-zero and
prints the daemon's PID rather than reporting success while it is still running; its
PID/socket files are left in place, and retrying (or checking `--status`) is safe.

The daemon is identified as shire's own by its executable — an exact match, a basename
starting with `shire` (a versioned install like `shire-v0.7`, a renamed download), or the
exact same file as the `shire` binary currently invoking `--stop`/`--status` — so a
renamed or versioned binary is recognized correctly rather than being refused and having
its live socket deleted out from under it.

## Smart filtering

The watch daemon avoids unnecessary rebuilds:

- **Edit/Write/NotebookEdit tools** — the changed file is checked for relevance against
  the same configuration the indexer itself uses: manifest filenames
  (`discovery.manifests`), source file extensions, doc extensions (`docs.extensions`),
  and `discovery.custom` rules. Paths under an excluded directory (`discovery.exclude`,
  e.g. `node_modules`) are skipped even if the extension would otherwise match.
- **Bash commands** — filtered against an *allowlist* of known read-only commands (`ls`,
  `git status`, `cargo test`, etc.) that are skipped; unknown commands default to
  triggering a rebuild.

## Troubleshooting

- `.shire/watch-stderr.log` — the daemon's stderr, including a bind failure (e.g. the
  socket path exceeding the platform's `SUN_LEN`, which is more likely on deeply nested
  repo paths). If `shire watch` fails to start, this file has the reason.
- `.shire/logs/shire.log.<date>` — the daemon's regular tracing output (set `SHIRE_LOG=debug`
  for verbose per-rebuild logging, including which files were skipped as irrelevant).
