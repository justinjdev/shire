# MCP Tools & Prompts

## Tools

Shire exposes the following tools over the Model Context Protocol:

| Tool | Description |
|---|---|
| `search_packages` | Search packages by name or description. Use instead of Grep for finding packages. |
| `list_packages` | List all indexed packages, optionally filtered by kind |
| `package_dependencies` | List a package's dependencies. Set `depth>1` for transitive graph (returns edge list with different schema; `limit` caps the edge list too). |
| `package_dependents` | Find all packages that depend on this package |
| `search_symbols` | Find functions, classes, types, methods by identifier or identifier prefix (not regex or substring). `handle` matches `handleRequest`; `verify jwt` matches `verifyJwtToken`. Matches the symbol name and its sub-tokens only, never signatures or file paths. Omit `query` with a `package` filter to list that package's symbols in (file, line) order, capped at `limit`. |
| `get_file_symbols` | List all symbols defined in a specific file. Use instead of reading the file to understand its exports. |
| `search_files` | Find files by path or name. Use instead of Glob/find for locating files. Useful for "middleware", "proto files", or files in a specific directory. |
| `search_docs` | Search documentation files by content, title, or path — returns matching docs with text snippets |
| `list_package_files` | List all files in a package, optionally filtered by extension. Use instead of Glob for listing package contents. |
| `explore` | Explore a concept across the codebase — searches packages, symbols, files, and documentation semantically. Use as the first tool when investigating unfamiliar code or broad topics like "authentication" or "error handling". Returns a structured context map organized by package. |
| `index_status` | Index build metadata: timestamp, git commit, counts |
| `symbol_references` | Find all references to a symbol by name. Returns `[{name, kind, file_path, line, package, enclosing_symbol}]`. Accepts optional `kind` and `package` filters. **Requires `symbols.references_enabled = true` (experimental, opt-in).** Note: matching is name-based. `enclosing_symbol` is dot-qualified (`AuthService.login`); a qualified name passed as `name` is resolved through `symbols.parent_symbol`, and references written in packages that define their own symbol of that name are left out — see [Qualified names](#qualified-names). |
| `symbol_callers` | List all callers of a symbol (call-site references). Returns `[{caller_name, caller_file, caller_line, caller_package, call_sites}]`, where `caller_name` is the dot-qualified enclosing path (`AuthService.login`) and can be fed straight back in as `name` — a qualified `name` is resolved through the type that defines the method (see [Qualified names](#qualified-names)). Accepts optional `package` filter. **Requires `symbols.references_enabled = true`.** Same name-based-match caveat as `symbol_references`. |
| `symbol_callees` | List what a function calls (outbound call graph). Returns `[{callee_name, first_file, first_line, call_sites}]`. Accepts a bare method name (`login`, which matches every qualified form such as `AuthService.login`) or a qualified one (`AuthService.login`, which matches only that method), plus an optional `package` filter. **Requires `symbols.references_enabled = true`.** |
| `change_impact` | Analyze the blast radius of changing a symbol. Combines cross-references with the dependency graph to return `{direct_impact, cross_package_impact, transitive_impact, summary}`. Use before renaming, changing a signature, or deleting a symbol. Accepts optional `package` (home package hint, for disambiguation), `transitive_depth` (default 2), and `limit`. **Requires `symbols.references_enabled = true`.** A dot-qualified `name` sets `home_package` from the type that defines it and reports `excluded_packages`; same name-based-match caveat as `symbol_references`. |
| `schema_consumers` | Find all files generated from a schema file (e.g. `.proto`). Returns generated file paths and their packages. Use to understand the blast radius of a schema change. |
| `generated_from` | Find the source schema file that generated a given file. Use to trace a generated file (e.g. `user.pb.go`) back to its source proto. |

### How matching works

All four search tools (`search_symbols`, `search_packages`, `search_files`,
`search_docs`) run the same FTS5 query builder:

- The query is split on whitespace and **every token must match** (implicit AND).
- Each token matches by **prefix**: `handle` matches `handleRequest` and
  `handle_request`. Tokens of one character are matched exactly instead —
  `packages_fts` and `docs_fts` index 2- and 3-character prefixes, while
  `symbols_fts` and `files_fts` deliberately carry no prefix index (a prefix
  query there walks a term range instead, measured as equally fast and ~28%
  smaller on disk), so a single-character prefix would have to scan every term.
- Each tool searches only the columns it is about. `search_symbols` matches
  the **symbol name and its sub-tokens** — not signatures, file paths or kinds
  (filter by kind with `kind`, find paths with `search_files`, and use Grep
  for text inside a signature). `search_files` matches the path,
  `search_packages` the package name, description and path, and `search_docs`
  the doc title, body and path.
- `search_symbols` orders exact name matches first, so searching `handle` —
  or `handle*`, or a pasted `handle.` — never buries a symbol actually called
  `handle` under its own prefixes.
- Symbol names are additionally indexed by their **sub-tokens**:
  `verifyJwtToken` is indexed as `verify`, `jwt`, `token`, so `verify jwt`,
  `jwt` and `token` all find it. This applies to symbol names only, not to
  file paths or doc bodies.
- Matching is by identifier, not regex or substring: `andleRequ` finds nothing.
- Operators in a query (`OR`, `NEAR`, `*`, `-`, `column:`) are treated as
  literal text, not as FTS5 syntax.

### Qualified names

`symbol_references`, `symbol_callers` and `change_impact` take a symbol name.
The reference index stores **bare** names (`run`), while `enclosing_symbol` /
`caller_name` come back dot-qualified (`AuthService.run`) and are meant to be
fed straight back in. A qualified name is resolved in three steps:

1. **Literally.** Some refs really are dot-named — an `import` of `os.path`.
   If the name matches refs as given, that is the answer.
2. **Through the qualifier.** Otherwise the last segment before the dot is
   looked up in `symbols.parent_symbol`: `A.run` finds the symbols named `run`
   whose parent is `A`, which is where the symbol lives. A reference row
   records the name and the package it was *written in*, never the type it
   resolves to, so the qualifier cannot filter references directly — what it
   can do is attribute them. A `run` written inside a package that defines its
   own `run` on some other type belongs to that package's method, so those
   packages are left out; every other package is kept, because a cross-package
   call site is exactly what these tools exist to find.
3. **Bare, and flagged.** If no indexed symbol carries that qualifier, the
   qualifier is dropped and the bare name is matched on its own.

Whenever the rows were matched on a name other than the one passed, the result
carries `matched_name` (the name actually matched), `matched_note` (what that
means) and, for step 2, `defined_in` and `excluded_packages`. Because those
fields need somewhere to live, a rewritten name always returns the single
object form described under [Result limits](#result-limits) — `results` plus
the match fields — even when nothing was truncated. `change_impact` already
returns an object, and gains `matched_name`, `qualifier_dropped` and
`excluded_packages`; it takes its `home_package` from the resolved symbol.

`excluded_packages` is worth reading before acting on a `change_impact`
answer: those packages were left out of `direct_impact`,
`cross_package_impact`, `summary.affected_packages` and the reverse-dep walk
seeded from it, so a call site in one of them is blast radius the qualifier
chose to attribute elsewhere. The list names every package that defines a
symbol of that name, not only the ones that turned out to reference it — most
entries will have had nothing to drop. `summary.excluded_ref_count` is the
number that matters: how many references those packages actually held. When it
is not zero, re-run with the bare name to see them.

A `package` filter is applied on top of the resolution, so asking for a
qualified name *and* a package that step 2 excluded is a contradiction and
returns nothing — `excluded_packages` in the response is what says why.

What this cannot do is separate two same-named methods **inside one package**:
with only the bare name recorded, `A.run` and `B.run` in the same package still
merge, and both are reported. Pass `package` to narrow the answer; use Grep
when the distinction has to be exact.

### Result limits

Tool output is pasted verbatim into a model's context, so every
list-returning tool is bounded:

| Tool | `limit` default | Maximum |
|---|---|---|
| `search_symbols`, `search_packages`, `search_files`, `search_docs` | 20 | 200 |
| `get_file_symbols`, `list_package_files`, `list_packages`, `package_dependencies`, `package_dependents`, `schema_consumers`, `generated_from` | 100 | 200 |
| `symbol_references`, `symbol_callers`, `symbol_callees`, `change_impact` | 100 | 200 |

`limit: 0` means "use the default", not "one row". The limit is applied in
SQL, and one row beyond it is fetched to tell a page that was cut from a list
that merely ends there.

A complete result is the bare JSON array. A truncated one — or, for the
reference tools, one whose name was rewritten (see
[Qualified names](#qualified-names)) — is a single JSON object instead:

```json
{"results": [...], "truncated": true, "limit": 20, "max": 200, "note": "showing the first 20 results …"}
```

so a capped list is never presented as a complete one, and a client that
concatenates the result's text blocks still gets parseable JSON. (At
`limit` = 200 the extra row cannot be fetched, so a result that fills the
ceiling is always flagged, with a `note` saying more rows *may* exist rather
than that they do — and the `note` then asks for a narrower request rather
than a bigger `limit`, which is already clamped at 200.)

`change_impact` returns an object rather than a list; when a bucket is capped
it gains the same `truncated` / `limit` / `max` / `note` fields.
`summary.direct_count` and `summary.cross_package_count` count every reference
scanned rather than only the rows returned — but the scan itself stops at
10 000 references, so they are totals only while `summary.counts_capped` is
false; when it is true they are floors. `summary.transitive_package_count` is
never a total: the reverse-dep walk stops at `limit`, so a capped result
reports a floor.

### Index freshness under `serve --root`

With `--root` the server reindexes on demand. Before answering a tool call it
checks how long ago the index was last built: inside the `serve.debounce_s`
window (default 5 seconds) it answers straight from the current index, and
outside it, it runs an incremental build first and answers from the result.

That build is the only freshness oracle — it compares the repo's file tree,
per-package mtimes and per-file content hashes itself, and costs on the order
of 60-200 ms when nothing has changed. So an ordinary working-tree edit is
picked up on the first tool call more than `serve.debounce_s` after it, with
no need to stage anything: `git add` and `.git/index` play no part.

Raise `serve.debounce_s` to trade freshness for fewer rebuilds during bursts
of tool calls; lower it for a repo where builds are cheap and edits frequent.

Without `--root` (plain `shire serve --db …`) the server is strictly
read-only and never rebuilds — refresh the index with `shire build`, the
watch daemon, or the `PostToolUse` hook.

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
