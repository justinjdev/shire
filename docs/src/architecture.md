# Architecture

```
src/
├── main.rs          # CLI (clap): build, serve, watch, rebuild, init, install, uninstall, clean subcommands
├── lib.rs           # Library re-exports for embedding shire as a crate
├── config.rs        # shire.toml parsing
├── git.rs           # Git worktree detection and repo root resolution
├── init.rs          # `shire init` setup (config, MCP server, hooks, rules)
├── install.rs       # `shire install`/`uninstall` — registers/removes shire as an MCP server across detected AI tools
├── logging.rs       # Rotating file logging (tracing-appender), per-session IDs
├── db/
│   ├── mod.rs       # SQLite schema, open/create
│   └── queries.rs   # FTS search, dependency graph BFS, listing
├── index/
│   ├── mod.rs       # Walk + incremental index orchestrator
│   ├── custom_discovery.rs # Config-driven custom package discovery
│   ├── manifest.rs  # ManifestParser trait
│   ├── hash.rs      # SHA-256 content hashing for incremental builds
│   ├── lock.rs      # Cross-process build lock (flock on <db_path>.lock)
│   ├── ref_writer.rs # Cross-reference write strategy threaded through the build phases
│   ├── npm.rs       # package.json parser (workspace: protocol)
│   ├── go.rs        # go.mod parser
│   ├── go_work.rs   # go.work parser (workspace use directives)
│   ├── cargo.rs     # Cargo.toml parser (workspace dep resolution)
│   ├── python.rs    # pyproject.toml parser
│   ├── maven.rs     # pom.xml parser (parent POM inheritance)
│   ├── gradle.rs    # build.gradle / build.gradle.kts parser
│   ├── gradle_settings.rs # settings.gradle parser (project inclusion)
│   ├── perl.rs      # cpanfile parser (requires, on 'test')
│   ├── ruby.rs      # Gemfile parser (gem, group blocks)
│   └── nix.rs       # flake.nix parser (inputs attrset, dotted and block forms)
├── symbols/
│   ├── mod.rs       # Symbol types, kind-agnostic extraction orchestrator
│   ├── walker.rs    # Source file discovery (extension filtering, excludes)
│   ├── registry.rs  # Language registry: maps extensions to tree-sitter grammars + hooks
│   ├── query_extract.rs # Generic tree-sitter query executor with hook callbacks
│   ├── queries/     # Tree-sitter .scm query files (one per language)
│   ├── hooks/       # Language-specific hooks (visibility, signatures, params, post-processing)
│   └── cobol.rs     # COBOL extractor (regex-based — the only non-tree-sitter language)
│                    # Cross-reference extraction (call, type, import, impl) is supported
│                    # for 8 tier-1 languages: Go, Python, Java, TypeScript, JavaScript,
│                    # Perl, Ruby, Scala. References are captured via @reference.* captures
│                    # in the language's .scm query and written to the symbol_refs table.
│                    # Coverage is asymmetric per language: JavaScript omits Type refs
│                    # (no type system), and Go/Perl omit Impl refs (no extends/implements).
├── mcp/
│   ├── mod.rs       # MCP server setup (rmcp, stdio transport)
│   ├── tools.rs     # 17 tool handlers
│   └── prompts.rs   # 2 prompt templates (explore, reference_audit)
├── watch/
│   ├── mod.rs       # Daemon event loop (UDS listener, debounce, rebuild)
│   ├── daemon.rs    # Process management (start/stop/is_running via PID)
│   └── protocol.rs  # Hook input parsing, Bash read-only allowlist
└── bin/
    └── autoresearch.rs # Benchmark harness, gated behind the non-default `bench` feature
```

## What an incremental build re-reads

A build re-hashes a package's source files when any of them changes **mtime,
size, or path**. Between them those cover the ordinary cases: an editor save
moves the mtime, a rename or a new/deleted file moves the path set, and a
restore that rewinds an mtime is still caught if the size differs.

One case is not covered: an edit that keeps the file's size *and* its mtime.
That is what `cp -p`/`install -p` out of a build cache, `rsync -a`, a `tar -x`
restore and some patch tools do. Nothing on disk distinguishes it from an
untouched file without reading every byte of the repo on every build, so
shire does not try — run `shire build --force` after one.

## When a walk cannot see the whole tree

Both walks a build makes — the repo-wide file walk and the per-package source
walk — distinguish "this is not there" from "I could not look". The difference
decides whether rows may be deleted.

An **unreadable path** (a directory whose permissions changed, a network share
that blipped, a container volume mid-remount) is recorded as a blind spot. Rows
under it are kept rather than deleted, no file-tree hash is stored (so the next
build re-checks the tree instead of short-circuiting), and a package whose
source walk failed keeps every symbol, reference and file hash it already had.
`shire build` reports that package and exits non-zero; the watch daemon and the
MCP server's on-demand rebuild only log it, because the build itself committed.

Hitting the **500,000-file cap** is the same thing: the walk stopped partway
through a nondeterministic traversal, so it is treated as a blind spot over the
whole tree — nothing is deleted, no file-tree hash is stored, and the build
prints a warning naming the cap. Exclude directories in `discovery.exclude` to
bring a repository back under it.

A malformed pattern in a committed `.gitignore` is *not* a blind spot: the
`ignore` crate reports it as an error but still walks the whole tree, so it is
warned about once and skipped.

## The build lock

Every build holds an advisory `flock` on `<db_path>.lock` for its whole run, so
two builders cannot both take the insert-only full-build path and double every
symbol. `shire build` and the watch daemon wait for a competing build (up to
`index::lock::LOCK_TIMEOUT`, minutes rather than seconds) and then report the
conflict; the MCP server's per-tool-call rebuild skips instead and logs it,
since its trigger comes round again.

`shire clean` takes the same lock before removing anything, and refuses with
"a shire build is running" rather than deleting a database out from under a
build. The `<db_path>.lock` file itself is left in place — it is an empty
sidecar like `-wal`/`-shm`, and unlinking it while a builder holds a lock on
that inode is what would let a second builder take the lock at the same path.

## symbol_refs table

The `symbol_refs` table stores cross-reference records extracted alongside symbol definitions. Each row captures a reference to a named symbol:

| Column | Type | Description |
|---|---|---|
| `name` | TEXT | The name being referenced (function, type, module, etc.) |
| `kind` | TEXT | One of: `call`, `type`, `import`, `impl` |
| `file_id` | INTEGER | `REFERENCES files(id) ON DELETE CASCADE` — the file containing the reference |
| `line` | INTEGER | Line number of the reference |
| `package` | TEXT | Package the referencing file belongs to (nullable) |
| `enclosing_symbol` | TEXT | Dot-qualified path of the enclosing scopes, innermost last (`AuthService.login`, `Outer.outer.Inner.run`); nullable |

`file_id` stores a compact reference into `files(id)` rather than a duplicated path string; `phase_index_files` runs before symbol extraction so `files` is already populated when refs are inserted, and read queries JOIN `files` to resolve `file_path`. B-tree indexes on `name`, `file_id`, and `enclosing_symbol` (plus composite and partial covering indexes for the callers/callees/package-scoped queries) support the exact-match lookups used by the `symbol_references`, `symbol_callers`, and `symbol_callees` MCP tools. No FTS5 table — reference queries are exact-name only.

Incremental behavior mirrors symbol extraction: references for a file are dropped and re-extracted whenever the file's SHA-256 hash changes. No separate pass is needed — references are extracted in the same tree-sitter walk as symbol definitions.
