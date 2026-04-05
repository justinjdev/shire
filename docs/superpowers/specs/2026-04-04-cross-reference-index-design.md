# Cross-Reference Index — Design

GitHub issue: [#84](https://github.com/justinjdev/shire/issues/84)

## Problem

Shire indexes symbol **definitions** but not **usages**. An AI using Shire can find where a function is defined, but cannot answer "who calls this?" or "what does this function call?" — the two load-bearing questions for refactor safety and call-graph navigation. This design adds a reference index alongside the existing symbol index and exposes it through three new MCP tools.

## Goals

- Index call sites, type references, and imports at file granularity
- Expose call-graph navigation through MCP tools (`symbol_callers`, `symbol_callees`)
- Expose raw reference lookup (`symbol_references`) for flexible agent-side analysis
- Reuse the existing tree-sitter, SQLite, FTS5, and incremental-rebuild infrastructure
- Ship a useful tier 1 language set; leave tier 2 as additive follow-ups

## Non-goals

- Dead-code detection (`unused_symbols`) — deferred to a follow-up
- Scoped or semantic name resolution (import-aware, type-aware)
- Call-graph transitive closure or cycle detection as MCP tools
- Tier 2 languages (Rust, C/C++, Swift, PHP, Kotlin, Dart, and the rest)

## Scope decisions

| Decision | Choice | Rationale |
|---|---|---|
| Name resolution | **Name-based (lexical)** | Matches issue phrasing; uniform across 30+ languages; false positives acceptable for AI consumption |
| Reference kinds | **Call + Type + Import** | 80% of value; imports are cheap to extract; excludes field/variable access |
| Language coverage | **Tier 1: Go, TS/JS, Python, Java, Perl, Ruby, Scala** | Targeted user workload; other languages unchanged |
| Query architecture | **Extend existing `.scm` files** | One parse, one pass, shared hook infrastructure |
| Enclosing symbol | **Tracked** | Required for `symbol_callers`/`symbol_callees` to be call-graph tools rather than grep |
| `unused_symbols` | **Out of scope** | Revisit if dead-code detection proves needed |

## Architecture

### New data type

```rust
// src/symbols/mod.rs
#[derive(Debug, Clone, Serialize)]
pub struct ReferenceInfo {
    pub name: String,                     // referenced symbol name
    pub kind: ReferenceKind,              // Call | Type | Import
    pub file_path: Arc<str>,
    pub line: usize,
    pub enclosing_symbol: Option<String>, // nearest containing fn/method/class
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind { Call, Type, Import }
```

### Extraction pipeline

Existing per-language `.scm` files gain new captures alongside `@definition.X`:

- `@reference.call` — function/method invocations
- `@reference.type` — identifier used in type position
- `@reference.import` — imported identifiers from import statements

`query_extract::extract()` splits matches by capture family. Definitions flow to `Vec<SymbolInfo>` as today; references flow to `Vec<ReferenceInfo>`. Signature change:

```rust
// src/symbols/mod.rs
pub fn extract_file(ext: &str, source: &str, file_path: Arc<str>)
    -> (Vec<SymbolInfo>, Vec<ReferenceInfo>);
```

Enclosing-symbol resolution walks `node.parent()` upward until it hits a per-language set of ancestor node kinds (e.g., `function_item`, `method_definition`, `class_declaration`). The ancestor set lives in `LanguageHooks` as a new field `enclosing_ancestors: &'static [&'static str]`.

Languages outside tier 1 return an empty reference vec — their existing queries and symbol extraction are unchanged.

### Noise filtering

Each tier 1 language carries a small stoplist of built-in names that should never be recorded as references (`true`, `false`, `nil`, `self`, `this`, `print`, language-specific keywords-that-parse-as-identifiers). The stoplist lives on `LanguageHooks` as `reference_stoplist: &'static [&'static str]` and is checked inside `query_extract::extract()` before a reference is emitted.

### Enclosing-symbol resolution for anonymous scopes

Arrow functions, lambdas, and other anonymous constructs have no name. The resolution walk skips them and continues upward until it finds a named ancestor (function, method, class). A reference inside `const foo = () => bar()` resolves its enclosing symbol to whatever named function/class contains `foo`, not to the anonymous arrow itself. If no named ancestor exists (top-level code), `enclosing_symbol` is `None`.

## Database schema

```sql
CREATE TABLE symbol_refs (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,              -- 'call' | 'type' | 'import'
    file_path TEXT NOT NULL,
    line INTEGER NOT NULL,
    package TEXT,
    enclosing_symbol TEXT
);

CREATE INDEX idx_refs_name ON symbol_refs(name);
CREATE INDEX idx_refs_file ON symbol_refs(file_path);
CREATE INDEX idx_refs_enclosing ON symbol_refs(enclosing_symbol);
```

Plus an FTS5 virtual table over `name` (same tokenizer as `symbols_fts`, triggers dropped/recreated during bulk writes).

Schema version bump in `shire_meta` triggers automatic migration — existing databases build the new table and FTS index at next `shire build`.

Expected size: references outnumber definitions roughly 5–10x in typical code. A 100k-symbol codebase produces ~500k–1M ref rows. Storage is cheap; the `name` index is what makes lookup fast.

## Incremental rebuild

Reuses the file-hash-triggered loop in `src/index/mod.rs`. No new tracking needed — references are file-scoped and regenerate with their owning file.

Per changed file, inside the existing transaction:

1. `DELETE FROM symbols WHERE file_path = ?` (existing)
2. `DELETE FROM symbol_refs WHERE file_path = ?` (new)
3. Bulk-insert new symbols (existing)
4. Bulk-insert new references (new)

File deletion propagates to `symbol_refs` the same way it propagates to `symbols`.

## MCP tools

### `symbol_references(name, kind?, package?, limit?)`

Flat list of raw references.

- `name` (required) — symbol name
- `kind` (optional) — `call` | `type` | `import`
- `package` (optional) — filter to refs inside a specific package
- `limit` (optional, default 100)

Returns: `[{ name, kind, file_path, line, package, enclosing_symbol }]`

### `symbol_callers(name, package?, limit?)`

Distinct enclosing symbols that contain a call to `name`. Navigates the call graph upward.

- `name` (required) — symbol being called
- `package` (optional) — restrict callers to a specific package
- `limit` (optional, default 100)

Returns: `[{ caller_name, caller_file, caller_line, call_sites }]` where `call_sites` is the count of calls from that caller.

### `symbol_callees(name, package?, limit?)`

Distinct names called from inside `name`. Navigates the call graph downward.

- `name` (required) — caller symbol
- `package` (optional)
- `limit` (optional, default 100)

Returns: `[{ callee_name, call_sites }]`.

### Prompt template

`reference_audit` in `src/mcp/prompts.rs` — guides an AI through a rename-safety analysis using `symbol_callers` + `symbol_references`.

## Tier 1 language query details

Exact grammar node names per language. Each gets a stoplist of built-in identifiers and a set of `enclosing_ancestors`.

| Language | Call | Type ref | Import | Enclosing ancestors |
|---|---|---|---|---|
| Go | `call_expression.function` | `type_identifier` | `import_spec.name` | `function_declaration`, `method_declaration` |
| TypeScript | `call_expression.function` | `type_identifier`, `type_reference` | `import_statement` members | `function_declaration`, `method_definition`, `class_declaration` (arrow_function skipped) |
| JavaScript | `call_expression.function` | N/A (no static types) | `import_statement` members | `function_declaration`, `method_definition`, `class_declaration` (arrow_function skipped) |
| Python | `call.function` | annotation `identifier` | `import_from_statement.name` | `function_definition`, `class_definition` |
| Java | `method_invocation.name` | `type_identifier` | `import_declaration.scoped_identifier` | `method_declaration`, `class_declaration`, `interface_declaration` |
| Perl | subroutine call nodes | N/A | `use_statement` | `subroutine_declaration_statement`, `package_statement` |
| Ruby | `call.method` | constant reference | `call` with method=`require` | `method`, `singleton_method`, `class`, `module` |
| Scala | `call_expression`, `infix_expression` | `type_identifier` | `import_declaration` | `function_definition`, `class_definition`, `object_definition`, `trait_definition` |

## Known limitations

- **Name collisions**: two packages defining `parse_config` cannot be distinguished — a reference to one matches both. Documented in MCP tool descriptions.
- **Import aliases**: `from foo import bar as baz` followed by `baz()` records a call-ref to `baz`, not `bar`. Aliased references will miss their definitions.
- **Generic/parameterized types**: `Vec<Foo>` records a ref to `Foo`, not to `Vec<Foo>`. This is probably the right behavior (we usually want to navigate to `Foo`), but generic-type navigation is not modeled.
- **Dynamic calls**: method calls on variables whose type is not statically known are recorded by method name only.

## Tests

- Per-language unit tests in `src/symbols/tests.rs` covering call/type/import captures and enclosing-symbol resolution across nested scopes
- Stoplist tests verifying built-ins are excluded
- Integration test in `tests/integration.rs`: build → MCP calls → assert `symbol_callers`/`symbol_callees`/`symbol_references` return expected rows
- Incremental test: modify file → verify stale refs removed, new refs inserted, unchanged files untouched

## Documentation updates

- `docs/src/mcp-tools.md` — 3 new tool entries
- `docs/src/architecture.md` — symbols module description updated to include references
- `docs/src/ecosystems.md` — new "Reference extraction" table showing per-language coverage
- `CLAUDE.md` — architecture section updated; add "adding a new reference extractor" workflow

## Follow-ups

- `unused_symbols` tool for dead-code detection
- Tier 2 language rollout (Rust, C/C++, Swift, PHP, Kotlin, Dart, Elixir, Erlang, Haskell, and the remaining tree-sitter languages)
- Scoped name resolution (import-aware) to reduce false positives from name collisions
- Call-graph traversal tools (transitive callers, cycle detection)
