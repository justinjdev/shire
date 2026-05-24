# Proto Boundary Mapping

**Issue:** #86 (scoped to protobuf only)
**Date:** 2026-04-05

## Problem

In polyglot monorepos, `.proto` files generate code in multiple languages (`*.pb.go`, `*_pb2.py`, `*.pb.ts`, etc.). Shire already extracts proto symbols (messages, services, RPCs) and skips generated files during symbol extraction. But there's no way to ask "what files were generated from this proto?" or "where does this generated file come from?" — the generation relationship is invisible.

## Scope

Protobuf only. GraphQL, OpenAPI, and FFI boundary detection are deferred to future work. No `change_impact` integration in this PR — the tools are standalone.

## Design

### Data model

New table in `src/db/mod.rs`:

```sql
CREATE TABLE IF NOT EXISTS boundary_edges (
    source_path       TEXT NOT NULL,
    generated_path    TEXT NOT NULL,
    source_package    TEXT,
    generated_package TEXT,
    kind              TEXT NOT NULL DEFAULT 'proto',
    PRIMARY KEY (source_path, generated_path)
);

CREATE INDEX IF NOT EXISTS idx_boundary_source ON boundary_edges(source_path);
CREATE INDEX IF NOT EXISTS idx_boundary_generated ON boundary_edges(generated_path);
```

`kind` is `'proto'` for now; column exists for future boundary types.

### Discovery algorithm

Piggybacks on the existing phase 4 file walk — no extra DB queries or file I/O.

**During the file walk** (as we iterate every file):

1. If extension is `proto` → extract stem from filename (e.g., `user` from `proto/api/v1/user.proto`), collect into `proto_map: HashMap<stem, Vec<(path, package)>>`
2. If filename matches a known generated suffix → extract stem by stripping the suffix, collect into `generated_map: HashMap<stem, Vec<(path, package)>>`

**Known generated suffixes** (reuse from `walker.rs` — single source of truth):

| Suffix | Language |
|---|---|
| `.pb.go` | Go |
| `_pb2.py` | Python |
| `_pb2_grpc.py` | Python gRPC |
| `.pb.h` | C/C++ header |
| `.pb.cc` | C/C++ |
| `.pb.ts` | TypeScript |
| `.pb.js` | JavaScript |
| `_pb.d.ts` | TypeScript declarations |
| `.pb.dart` | Dart |
| `_pb.rb` | Ruby |

**After the walk completes:**

3. Intersect `proto_map` and `generated_map` on stem.
4. For each `(proto, generated)` pair, apply **scope filter** — accept the pair if any of these hold:
   - **Same package**: `proto.package == generated.package`
   - **Dependent package**: `generated.package` declares a dependency on `proto.package` (join `dependencies` table)
   - **Sibling package**: `proto.package` and `generated.package` share a parent directory (one level up from their package paths)
5. Batch insert accepted pairs into `boundary_edges`.

**Incremental behavior:** Boundary edges are cheap to compute (in-memory map intersection after the file walk that already happens). Rebuild the full `boundary_edges` table on every index build — truncate then batch insert. No per-package diffing needed; the walk is already incremental at the file level, and the insert is a single batch operation.

### Why scope filtering matters

Common stems like `api`, `common`, `service` appear in many proto files across a monorepo. Without scope filtering, `api.proto` in `services/auth/` would match `api.pb.go` in `services/billing/` — a false positive. Scope filtering (same package → dependent → sibling) ensures high precision. Missing a real edge is better than recording a false one.

### MCP tools

Two new tools in `src/mcp/tools.rs`:

**`schema_consumers`** — given a schema file path, return all generated files and their packages.

```
Input:  { path: String }
Output: [{ generated_path: String, generated_package: Option<String>, kind: String }]
Query:  SELECT generated_path, generated_package, kind
        FROM boundary_edges WHERE source_path = ?
```

**`generated_from`** — given a generated file path, return the source schema.

```
Input:  { path: String }
Output: [{ source_path: String, source_package: Option<String>, kind: String }]
Query:  SELECT source_path, source_package, kind
        FROM boundary_edges WHERE generated_path = ?
```

Both are indexed lookups. No `refs_disabled` gate — boundary edges are independent of the cross-reference index.

### Files touched

| File | Change |
|---|---|
| `src/db/mod.rs` | Add `boundary_edges` table + indexes to schema |
| `src/db/queries.rs` | Add `BoundaryEdge` struct, `insert_boundary_edges`, `delete_boundary_edges_for_package`, `query_schema_consumers`, `query_generated_from` |
| `src/index/mod.rs` | Add phase 6 boundary detection: collect proto/generated files during walk, match stems, scope-filter, batch insert |
| `src/symbols/walker.rs` | Extract generated suffix list into a public constant (currently hardcoded in `is_generated_file`) so the boundary detector can reuse it |
| `src/mcp/tools.rs` | Add `schema_consumers` and `generated_from` tools + args structs |
| `docs/src/mcp-tools.md` | Add tool rows |
| `CLAUDE.md` | Bump tool count (15 → 17), add boundary detection to index/ and mcp/ descriptions |

### Not in scope

- `change_impact` integration (future PR)
- GraphQL / OpenAPI / FFI boundary types
- `buf.gen.yaml` parsing for precise output directory resolution
- Boundary edges for non-proto codegen (e.g., OpenAPI → client SDK)
