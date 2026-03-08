# MCP Tools & Prompts

## Tools

Shire exposes the following tools over the Model Context Protocol:

| Tool | Description |
|---|---|
| `search_packages` | Search packages by name or description |
| `list_packages` | List all indexed packages, optionally filtered by kind |
| `package_dependencies` | List a package's dependencies (set `depth>1` for transitive graph) |
| `package_dependents` | Find all packages that depend on this package |
| `search_symbols` | Search symbols by name or signature; supports hybrid FTS + vector search when [RAG is enabled](./configuration.md#rag-vector-search) |
| `get_file_symbols` | List all symbols defined in a specific file |
| `search_files` | Search files by path or name using full-text search |
| `list_package_files` | List all files in a package, optionally filtered by extension |
| `explore` | Semantic codebase exploration — search packages, symbols, and files for a concept |
| `index_status` | Index build metadata: timestamp, git commit, counts |

## Prompts

Prompts are pre-built templates that compose multiple queries into structured context. They give your AI a map of where concepts live in the codebase.

| Prompt | Args | Description |
|---|---|---|
| `explore` | `query` | Search packages, symbols, and files for a concept — returns a structured context map organized by package |
