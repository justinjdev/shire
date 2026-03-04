# MCP Tools

Shire exposes the following tools over the Model Context Protocol:

| Tool | Description |
|---|---|
| `search_packages` | Full-text search across package names, descriptions, and paths |
| `get_package` | Exact name lookup for a single package |
| `list_packages` | List all packages, optionally filtered by kind |
| `package_dependencies` | What a package depends on (optionally internal-only; set `depth=N` for transitive BFS traversal) |
| `package_dependents` | Reverse lookup — what depends on this package |
| `search_symbols` | Full-text search across symbol names and signatures |
| `get_package_symbols` | List all symbols in a package (functions, classes, types, methods) |
| `get_file_symbols` | List all symbols defined in a specific file |
| `list_package_files` | List all files belonging to a package, with optional extension filter |
| `index_status` | When the index was built, git commit, package/symbol/file counts, build duration |
