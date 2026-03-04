# MCP Tools

Shire exposes the following tools over the Model Context Protocol:

| Tool | Description |
|---|---|
| `search_packages` | Search packages by name or description |
| `list_packages` | List all indexed packages, optionally filtered by kind |
| `package_dependencies` | List a package's dependencies (set `depth>1` for transitive graph) |
| `package_dependents` | Find all packages that depend on this package |
| `search_symbols` | Search symbols by name or signature; supports hybrid FTS + vector search when [RAG is enabled](./configuration.md#rag-vector-search) |
| `get_file_symbols` | List all symbols defined in a specific file |
| `list_package_files` | List all files in a package, optionally filtered by extension |
| `index_status` | Index build metadata: timestamp, git commit, counts |
