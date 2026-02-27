## Context

Symbol extraction is currently kind-scoped: `extract_symbols_for_package()` calls `extensions_for_kind(package_kind)` to determine which file extensions to scan, then dispatches to language-specific extractors based on extension. This means a Gradle package only scans `.java`/`.kt`, and a proto file sitting in that directory is invisible.

The existing architecture is clean — `extensions_for_kind` maps package kinds to extensions, `walk_source_files` filters by those extensions, and the match block in `extract_symbols_for_package` dispatches to extractors. The change needs to widen which extensions get scanned without breaking the existing per-kind model for packages that only want their own language.

## Goals / Non-Goals

**Goals:**
- Add tree-sitter proto symbol extraction (messages, services, RPCs, enums)
- Make symbol extraction kind-agnostic by default — scan all registered extensions for every package
- Allow users to exclude specific file extensions from symbol extraction via config
- Proto symbols attach to whatever package owns the directory (no proto-specific package discovery)

**Non-Goals:**
- Proto package discovery or manifest parsing
- Proto dependency parsing (imports between proto files)
- Changing how packages are discovered or associated with directories
- Modifying the `SymbolKind` enum (proto constructs map to existing kinds: `struct` for messages, `interface` for services, `method` for RPCs, `enum` for enums)

## Decisions

### 1. Kind-agnostic extraction via `all_extensions()` function

Replace the call to `extensions_for_kind(package_kind)` in `extract_symbols_for_package` with a new `all_extensions()` function that returns every registered extension. The `exclude_extensions` config filters the result.

**Why not keep kind-scoped and add proto as a secondary pass?** Two passes over the same directory tree is wasteful, and every new language would need the same workaround. Kind-agnostic is simpler and handles all future cases.

**Why not remove `extensions_for_kind` entirely?** It's still useful for `custom-package-discovery` where users can override extensions per rule. Keep it but stop using it as the default path.

### 2. Proto symbol kind mapping

Map proto constructs to existing `SymbolKind` variants rather than adding new ones:

| Proto construct | SymbolKind | Rationale |
|---|---|---|
| `message` | `Struct` | Data structure, closest match |
| `service` | `Interface` | Defines a contract |
| `rpc` | `Method` | Operation on a service, with `parent_symbol` = service name |
| `enum` | `Enum` | Direct match |
| `oneof` | `Type` | Named union type within a message |

**Alternative:** Add proto-specific kinds (`Message`, `Service`, `Rpc`). Rejected because it would require changes to the DB schema, query layer, and MCP server for marginal benefit — the existing kinds convey the right semantics.

### 3. Config shape: `[symbols].exclude_extensions`

```toml
[symbols]
exclude_extensions = [".proto", ".pl"]
```

A deny-list rather than an allow-list. Rationale: new languages should work by default when their extractors are added. An allow-list would require users to update config every time Shire adds a language. Extensions include the dot prefix for clarity.

### 4. tree-sitter-protobuf crate

Use the `tree-sitter-protobuf` crate for parsing. If unavailable or poorly maintained, fall back to a simple regex-based extractor — proto syntax is regular enough that `message Foo {`, `service Bar {`, `rpc Baz(` patterns are reliable.

## Risks / Trade-offs

**[Performance] Scanning all extensions for every package** → Mitigation: `walk_source_files` already skips non-matching files efficiently via extension set check. Adding more extensions to the set has negligible cost since the directory walk itself dominates. The filter is O(1) per file.

**[Noise] Packages gain unexpected symbols from other languages** → Mitigation: `exclude_extensions` config gives users control. This is also the desired behavior — a Gradle service with proto files *should* show proto symbols.

**[Compatibility] Existing indexes change on rebuild** → Mitigation: Additive only — new symbols appear, no existing symbols change or disappear. Users who don't want this can exclude extensions.
