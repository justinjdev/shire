# MCP Tools

Shire exposes the following tools over the Model Context Protocol:

| Tool | Description |
|---|---|
| `search_packages` | Full-text search across package names, descriptions, and paths |
| `get_package` | Exact name lookup for a single package |
| `list_packages` | List all packages, optionally filtered by kind |
| `package_dependencies` | What a package depends on (optionally internal-only) |
| `package_dependents` | Reverse lookup — what depends on this package |
| `dependency_graph` | Transitive BFS traversal from a root package |
| `search_symbols` | Full-text search across symbol names and signatures |
| `get_package_symbols` | List all symbols in a package (functions, classes, types, methods) |
| `get_symbol` | Exact name lookup for a symbol across packages |
| `get_file_symbols` | List all symbols defined in a specific file |
| `search_files` | Full-text search across file paths, with optional package/extension filter |
| `list_package_files` | List all files belonging to a package, with optional extension filter |
| `index_status` | When the index was built, git commit, package/symbol/file counts, build duration |
