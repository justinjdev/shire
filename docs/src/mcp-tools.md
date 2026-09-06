# MCP Tools & Prompts

## Tools

Shire exposes the following tools over the Model Context Protocol:

| Tool | Description |
|---|---|
| `search_packages` | Search packages by name or description. Use instead of Grep for finding packages. |
| `list_packages` | List all indexed packages, optionally filtered by kind |
| `package_dependencies` | List a package's dependencies. Set `depth>1` for transitive graph (returns edge list with different schema). |
| `package_dependents` | Find all packages that depend on this package |
| `search_symbols` | Find functions, classes, types, methods by identifier or identifier prefix (not regex or substring). `handle` matches `handleRequest`; `verify jwt` matches `verifyJwtToken`. Omit query with a package filter to list the start of that package in (file, line) order. Supports hybrid FTS + vector search when [RAG is enabled](./configuration.md#rag-vector-search). |
| `get_file_symbols` | List all symbols defined in a specific file. Use instead of reading the file to understand its exports. |
| `search_files` | Find files by path or name. Use instead of Glob/find for locating files. Useful for "middleware", "proto files", or files in a specific directory. |
| `search_docs` | Search documentation files by content, title, or path — returns matching docs with text snippets |
| `list_package_files` | List all files in a package, optionally filtered by extension. Use instead of Glob for listing package contents. |
| `explore` | Explore a concept across the codebase — searches packages, symbols, files, and documentation semantically. Use as the first tool when investigating unfamiliar code or broad topics like "authentication" or "error handling". Returns a structured context map organized by package. |
| `index_status` | Index build metadata: timestamp, git commit, counts |
| `symbol_references` | Find all references to a symbol by name. Returns `[{name, kind, file_path, line, package, enclosing_symbol}]`. Accepts optional `kind` and `package` filters. **Requires `symbols.references_enabled = true` (experimental, opt-in).** Note: matching is name-based — same-name symbols across different packages are merged. |
| `symbol_callers` | List all callers of a symbol (call-site references). Returns `[{caller_name, caller_file, caller_line, call_sites}]`. Accepts optional `package` filter. **Requires `symbols.references_enabled = true`.** Same name-based-match caveat as `symbol_references`. |
| `symbol_callees` | List what a function calls (outbound call graph). Returns `[{callee_name, call_sites}]`. Accepts optional `package` filter. **Requires `symbols.references_enabled = true`.** Same name-based-match caveat as `symbol_references`. |
| `change_impact` | Analyze the blast radius of changing a symbol. Combines cross-references with the dependency graph to return `{direct_impact, cross_package_impact, transitive_impact, summary}`. Use before renaming, changing a signature, or deleting a symbol. Accepts optional `package` (home package hint, for disambiguation), `transitive_depth` (default 2), and `limit`. **Requires `symbols.references_enabled = true`.** Same name-based-match caveat as `symbol_references`. |
| `schema_consumers` | Find all files generated from a schema file (e.g. `.proto`). Returns generated file paths and their packages. Use to understand the blast radius of a schema change. |
| `generated_from` | Find the source schema file that generated a given file. Use to trace a generated file (e.g. `user.pb.go`) back to its source proto. |

### How matching works

All four search tools (`search_symbols`, `search_packages`, `search_files`,
`search_docs`) run the same FTS5 query builder:

- The query is split on whitespace and **every token must match** (implicit AND).
- Each token matches by **prefix**: `handle` matches `handleRequest` and
  `handle_request`. Tokens of one character are matched exactly instead —
  the prefix indexes cover 2- and 3-character prefixes, so a single-character
  prefix would have to scan every term.
- Symbol names are additionally indexed by their **sub-tokens**:
  `verifyJwtToken` is indexed as `verify`, `jwt`, `token`, so `verify jwt`,
  `jwt` and `token` all find it. This applies to symbol names only, not to
  file paths or doc bodies.
- Matching is by identifier, not regex or substring: `andleRequ` finds nothing.
- Operators in a query (`OR`, `NEAR`, `*`, `-`, `column:`) are treated as
  literal text, not as FTS5 syntax.

### Result limits

Tool output is pasted verbatim into a model's context, so every
list-returning tool is bounded:

| Tool | `limit` default | Maximum |
|---|---|---|
| `search_symbols`, `search_packages`, `search_files`, `search_docs` | 20 | 200 |
| `get_file_symbols`, `list_package_files`, `list_packages`, `package_dependencies`, `package_dependents` | 100 | 200 |
| `symbol_references`, `symbol_callers`, `symbol_callees`, `change_impact` | 100 | 1000 |

The limit is applied in SQL. When a result fills the limit exactly, the
response carries a second text block saying so, with a hint for narrowing the
query — a capped list is never presented as a complete one.

### When to use Shire vs Grep/Glob

| Task | Use | Not |
|---|---|---|
| Find a function, class, or type by name | `search_symbols` | Grep |
| Find a file by name or path | `search_files` | Glob / find |
| List files in a package | `list_package_files` | Glob |
| Find a package | `search_packages` | Grep |
| Explore an unfamiliar area | `explore` | multiple Grep calls |
| Search for a literal string or log message | Grep | Shire |
| Search inside function bodies | Grep | Shire |
| Pattern match on file contents | Grep | Shire |

## Prompts

Prompts are pre-built templates that compose multiple queries into structured context. They give your AI a map of where concepts live in the codebase.

| Prompt | Args | Description |
|---|---|---|
| `explore` | `query` | Search packages, symbols, files, and documentation for a concept — returns a structured context map organized by package |
| `reference_audit` | `name` | Guides refactor-safety analysis for a symbol: classifies refs by kind, traces the call graph via `symbol_callers`, identifies cross-package impact, and assesses rename/change risk. **Requires `symbols.references_enabled = true` (experimental).** |
