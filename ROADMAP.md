# Shire Roadmap

## Shipped

- [x] **Incremental indexing** — SHA-256 content hashing + mtime pre-checks skip unchanged manifests and source files.
- [x] **Workspace-aware parsing** — npm workspaces, Cargo workspaces, `go.work` files.
- [x] **Worktree awareness** — Linked git worktrees share/seed a DB per repo, scoped by worktree (see `docs/src/worktrees.md`).
- [x] **Maven/Gradle/Ruby ecosystems** — `pom.xml`, `build.gradle`(`.kts`), `settings.gradle`(`.kts`), `Gemfile`.
- [x] **Watch mode** — Background daemon (`shire watch`) that rebuilds on a signal from the Claude Code `PostToolUse` hook or `shire rebuild` (see `docs/src/watch-daemon.md`).

## Open

- [ ] **`shire query` CLI** — Direct terminal access to search, deps, graph without spinning up the MCP server.
- [ ] **Export** — DOT/Mermaid graph output for visualization, JSON dump for pipelines.
- [ ] **Cycle detection** — Surface dependency cycles in the graph.
- [ ] **Package health queries** — Orphan packages, most-depended-on, unused internal deps.
- [ ] **More ecosystems** — .NET csproj, Swift Package.swift.
- [ ] **CI integration** — Detect dependency changes in PRs, enforce policies.
