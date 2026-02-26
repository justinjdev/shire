## Why

Shire answers "what packages exist and how they relate" but not "where is `AuthService` defined" or "what does `processPayment` accept." AI agents currently spend 5-10 minutes grepping across hundreds of files to locate symbols. A symbol index lets them go from query to exact file:line in one MCP call.

## What Changes

- **New `symbols` table** in SQLite storing extracted symbols with full signatures, parameters, return types, and file locations
- **FTS5 index on symbols** for fast name/signature search
- **tree-sitter AST parsing** for TypeScript/JavaScript, Go, Rust, and Python — extracting public/exported symbols from source files within each indexed package
- **3 new MCP tools**: `search_symbols`, `get_package_symbols`, `get_symbol`
- **Source file walking** within package directories to discover parseable files (.ts, .tsx, .js, .go, .rs, .py)
- **Incremental integration**: symbols re-extracted for packages whose manifests changed; `--force` clears symbol data too

## Capabilities

### New Capabilities
- `symbol-extraction`: tree-sitter-based AST parsing to extract public symbols (functions, classes, structs, interfaces, types, enums, traits, methods, constants) with full signatures from source files
- `symbol-querying`: MCP tools for searching, listing, and looking up symbols across the index

### Modified Capabilities
- `mcp-server`: 3 new tools added (search_symbols, get_package_symbols, get_symbol)
- `package-discovery`: Source files within package directories are scanned for symbol extraction after manifest parsing

## Impact

- `Cargo.toml` — new dependencies: `tree-sitter`, `tree-sitter-typescript`, `tree-sitter-javascript`, `tree-sitter-go`, `tree-sitter-rust`, `tree-sitter-python`
- `src/db/mod.rs` — new `symbols` table, `symbols_fts` virtual table, triggers
- `src/db/queries.rs` — new query functions for symbol search, listing, lookup
- `src/symbols/` — new module: source file walker, tree-sitter extractors per language, symbol types
- `src/index/mod.rs` — call symbol extraction after package parsing phase
- `src/mcp/tools.rs` — 3 new tool handlers
- DB schema version bump (additive — new tables only, no migration needed)
