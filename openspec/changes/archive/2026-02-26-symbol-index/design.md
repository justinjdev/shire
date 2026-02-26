# Symbol Index — Design

## Context

Shire indexes packages and dependencies but nothing about what's inside them. AI agents spend significant time grepping for function/class/type definitions. Adding a symbol index with full AST data (signatures, parameters, return types) lets agents go from "where is rate limiting" to an exact file:line with one MCP call.

## Goals / Non-Goals

**Goals:**
- Extract public/exported symbols from source files in all 4 supported ecosystems
- Store full signatures, parameters (name + type), and return types
- FTS5 search across symbol names and signatures
- 3 new MCP tools for symbol lookup
- Incremental: re-extract symbols when packages change

**Non-Goals:**
- Private/internal symbol extraction (only public API)
- Cross-file type resolution (e.g., resolving imported types)
- Symbol-level dependency graph (which symbol calls which)
- Lock file or build output parsing
- Supporting additional languages beyond the current 4

## Decisions

### Decision 1: tree-sitter for AST parsing

tree-sitter provides fast, fault-tolerant parsing for all 4 languages with Rust bindings. Each language grammar is a separate crate.

**Dependencies:**
- `tree-sitter = "0.24"` (core)
- `tree-sitter-typescript = "0.23"` (includes JS grammar)
- `tree-sitter-go = "0.23"`
- `tree-sitter-rust = "0.23"`
- `tree-sitter-python = "0.23"`

**Why tree-sitter over regex/line-based parsing:** Signatures span multiple lines, have nested types, and vary by language. Regex would be fragile. tree-sitter gives us a real syntax tree and handles partial/broken files gracefully.

### Decision 2: Schema — new `symbols` table

```sql
CREATE TABLE IF NOT EXISTS symbols (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    package       TEXT NOT NULL REFERENCES packages(name),
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,  -- function, class, struct, interface, type, enum, trait, method, constant
    signature     TEXT,
    file_path     TEXT NOT NULL,
    line          INTEGER NOT NULL,
    visibility    TEXT NOT NULL DEFAULT 'public',
    parent_symbol TEXT,
    return_type   TEXT,
    parameters    TEXT  -- JSON: [{"name": "x", "type": "i32"}, ...]
);

CREATE INDEX IF NOT EXISTS idx_symbols_package ON symbols(package);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);

CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name, kind, signature, file_path,
    content='symbols',
    content_rowid='rowid'
);
```

Content-synced FTS5 with triggers (same pattern as `packages_fts`).

### Decision 3: Module structure — `src/symbols/`

```
src/symbols/
├── mod.rs          # SymbolInfo struct, extract_symbols_for_package() orchestrator
├── walker.rs       # Walk package dir for source files by extension
├── typescript.rs   # TS/JS extraction using tree-sitter-typescript
├── go.rs           # Go extraction using tree-sitter-go
├── rust.rs         # Rust extraction using tree-sitter-rust
└── python.rs       # Python extraction using tree-sitter-python
```

Each language module exports: `fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo>`

The `SymbolInfo` struct:
```rust
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,  // enum: Function, Class, Struct, Interface, Type, Enum, Trait, Method, Constant
    pub signature: Option<String>,
    pub file_path: String,
    pub line: usize,
    pub visibility: String,
    pub parent_symbol: Option<String>,
    pub return_type: Option<String>,
    pub parameters: Option<Vec<Parameter>>,
}

pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<String>,
}
```

### Decision 4: Integration with build pipeline

Symbol extraction runs as a new phase after package parsing:

```
Phase 1: Walk manifests
Phase 1.5: Collect workspace context
Phase 2: Diff hashes
Phase 3: Parse manifests
Phase 4: Remove deleted packages
Phase 5: Recompute is_internal
Phase 6: Update manifest hashes
Phase 7: Extract symbols (NEW)  ← for each new/changed package
```

For packages that were parsed (new or changed), walk their source files and extract symbols. Clear old symbols for those packages first (`DELETE FROM symbols WHERE package = ?`), then insert new ones.

For deleted packages, symbols are removed via CASCADE or explicit delete.

### Decision 5: Per-language extraction approach

Each extractor uses tree-sitter queries to find relevant nodes:

- **TypeScript/JS**: Walk top-level `export_statement` nodes. For `export function`, `export class`, `export interface`, `export type`, `export enum`, `export const`. For class methods, walk into class body.
- **Go**: Walk top-level `function_declaration` and `type_declaration` nodes. Only include names starting with uppercase (exported). For methods, check `method_declaration` nodes and extract the receiver type.
- **Rust**: Walk `function_item`, `struct_item`, `enum_item`, `trait_item`, `impl_item` nodes. Only include those with `visibility_modifier` (`pub`). For impl methods, track the impl target type as parent.
- **Python**: Walk top-level `function_definition` and `class_definition` nodes. For class methods, walk into class body and filter by name (skip `_` prefix except `__init__`).

Signatures are extracted by reading the source text from the node's byte range (trimming the body/block).

### Decision 6: Incremental behavior

- Symbols are tied to packages. When a package's manifest changes (detected in phase 2-3), its symbols are cleared and re-extracted.
- `--force` also re-extracts all symbols.
- No per-source-file hashing in v1 — we re-extract all symbols for a changed package. This is fast enough since tree-sitter parses at ~MB/s speeds.
- Future: per-file source hashing for finer granularity.

## Approach

1. Add tree-sitter dependencies to Cargo.toml
2. Create `src/symbols/` module with shared types (SymbolInfo, SymbolKind, Parameter)
3. Implement source file walker (filter by extension, skip excluded dirs)
4. Implement each language extractor (TS, Go, Rust, Python)
5. Add `symbols` + `symbols_fts` tables to schema
6. Add symbol query functions to `db/queries.rs`
7. Integrate extraction into `build_index` as phase 7
8. Add 3 new MCP tools
9. Tests per extractor + integration tests
