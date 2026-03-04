# MCP Tools

Shire exposes the following tools over the Model Context Protocol:

| Tool | Description |
|---|---|
| `search_packages` | Search packages by name or description |
| `get_package` | Get full details for a specific package by exact name |
| `list_packages` | List all indexed packages, optionally filtered by kind |
| `package_dependencies` | List a package's dependencies (set `depth>1` for transitive graph) |
| `package_dependents` | Find all packages that depend on this package |
| `dependency_graph` | Get the transitive dependency graph starting from a package |
| `search_symbols` | Search symbols by name or signature; supports hybrid FTS + vector search when [RAG is enabled](./configuration.md#rag-vector-search) |
| `get_package_symbols` | List all symbols in a package — its exported functions, classes, types, and methods |
| `get_file_symbols` | List all symbols defined in a specific file |
| `search_files` | Search files by path or name using full-text search |
| `list_package_files` | List all files in a package, optionally filtered by extension |
| `explore` | Semantic codebase exploration — search packages, symbols, and files for a concept |
| `explore_package` | Deep dive into a specific package — metadata, dependencies, dependents, public API surface, and file tree |
| `impact_analysis` | Analyze blast radius — what breaks if this package changes? Shows direct and transitive dependents |
| `index_status` | Index build metadata: timestamp, git commit, counts |
