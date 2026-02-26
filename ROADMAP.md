# Shire Roadmap

## Near Term

- [ ] **Incremental indexing** — Track file mtimes or git diff to skip unchanged manifests instead of full rebuild. Critical for large repos.
- [ ] **`shire query` CLI** — Direct terminal access to search, deps, graph without spinning up the MCP server.
- [ ] **Export** — DOT/Mermaid graph output for visualization, JSON dump for pipelines.

## Medium Term

- [ ] **Workspace-aware parsing** — npm workspaces, Cargo workspaces, Go work files. Currently each manifest is parsed in isolation.
- [ ] **Cycle detection** — Surface dependency cycles in the graph.
- [ ] **Package health queries** — Orphan packages, most-depended-on, unused internal deps.

## Longer Term

- [ ] **Worktree awareness** — Detect and handle git worktrees; index packages across linked worktrees or scope indexing to the current worktree.
- [ ] **More ecosystems** — Maven/Gradle, .NET csproj, Ruby Gemfile, Swift Package.swift.
- [ ] **Watch mode** — File watcher that re-indexes on manifest changes.
- [ ] **CI integration** — Detect dependency changes in PRs, enforce policies.
