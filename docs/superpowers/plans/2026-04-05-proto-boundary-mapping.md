# Proto Boundary Mapping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Track `.proto` → generated-code relationships so users can query "what files were generated from this proto?" and vice versa.

**Architecture:** New `boundary_edges` table populated during the file-walk phase of `build_index`. Two new MCP tools (`schema_consumers`, `generated_from`) do indexed lookups against this table. Generated-file suffix patterns are extracted from `walker.rs` into a shared constant so both the walker skip logic and the boundary detector use a single source of truth.

**Tech Stack:** Rust, SQLite, rmcp (MCP server)

---

### Task 1: Extract generated-suffix constants from walker.rs

**Files:**
- Modify: `src/symbols/walker.rs:18-37`

- [ ] **Step 1: Write the test**

Add a test in `src/symbols/walker.rs` that asserts the proto-related generated suffixes are accessible via the new public constant:

```rust
#[test]
fn test_proto_generated_suffixes_non_empty() {
    assert!(!PROTO_GENERATED_SUFFIXES.is_empty());
    assert!(PROTO_GENERATED_SUFFIXES.contains(&".pb.go"));
    assert!(PROTO_GENERATED_SUFFIXES.contains(&"_pb2.py"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib symbols::walker::tests::test_proto_generated_suffixes_non_empty`
Expected: FAIL — `PROTO_GENERATED_SUFFIXES` not found.

- [ ] **Step 3: Extract the constant**

In `src/symbols/walker.rs`, add a new public constant containing only the proto-related generated suffixes. Keep `SKIP_SUFFIXES` as-is (it includes non-proto entries like `.generated.go`, `.d.ts`, `_test.go`).

```rust
/// Proto-specific generated file suffixes. Used by both the symbol walker
/// (to skip generated files) and the boundary detector (to match proto
/// sources to their generated outputs). Single source of truth.
pub const PROTO_GENERATED_SUFFIXES: &[&str] = &[
    ".pb.go",
    "_pb2.py",
    "_pb2_grpc.py",
    ".pb.h",
    ".pb.cc",
    ".pb.ts",
    ".pb.js",
    "_pb.d.ts",
    ".pb.dart",
    "_pb.rb",
];
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib symbols::walker::tests::test_proto_generated_suffixes_non_empty`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/symbols/walker.rs
git commit -m "refactor: extract PROTO_GENERATED_SUFFIXES from walker.rs"
```

---

### Task 2: Add boundary_edges table to DB schema

**Files:**
- Modify: `src/db/mod.rs:52-260` (inside `create_schema`)

- [ ] **Step 1: Write the test**

Add a test in `src/db/mod.rs` that verifies the table exists after schema creation:

```rust
#[test]
fn test_boundary_edges_table_exists() {
    let conn = Connection::open_in_memory().unwrap();
    create_schema(&conn).unwrap();
    // Insert and query a boundary edge — verifies table + indexes exist
    conn.execute(
        "INSERT INTO boundary_edges (source_path, generated_path, source_package, generated_package, kind) \
         VALUES ('a.proto', 'a.pb.go', 'pkg', 'pkg', 'proto')",
        [],
    ).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM boundary_edges WHERE source_path = 'a.proto'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib db::tests::test_boundary_edges_table_exists`
Expected: FAIL — table does not exist.

- [ ] **Step 3: Add table to schema**

In `src/db/mod.rs` `create_schema`, add after the `symbol_refs` indexes (line ~258):

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

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib db::tests::test_boundary_edges_table_exists`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/db/mod.rs
git commit -m "feat: add boundary_edges table to schema"
```

---

### Task 3: Add boundary edge query functions to db/queries.rs

**Files:**
- Modify: `src/db/queries.rs`

- [ ] **Step 1: Write the tests**

Add tests in `src/db/queries.rs` in a new `boundary_tests` module at the end of the file:

```rust
#[cfg(test)]
mod boundary_tests {
    use super::*;
    use crate::db::open_or_create;
    use tempfile::tempdir;

    #[test]
    fn test_insert_and_query_schema_consumers() {
        let dir = tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("b.db"), false).unwrap();

        let edges = vec![
            BoundaryEdge {
                source_path: "proto/user.proto".into(),
                generated_path: "gen/user.pb.go".into(),
                source_package: Some("proto-pkg".into()),
                generated_package: Some("go-pkg".into()),
                kind: "proto".into(),
            },
            BoundaryEdge {
                source_path: "proto/user.proto".into(),
                generated_path: "gen/user_pb2.py".into(),
                source_package: Some("proto-pkg".into()),
                generated_package: Some("py-pkg".into()),
                kind: "proto".into(),
            },
        ];
        batch_insert_boundary_edges(&conn, &edges).unwrap();

        let consumers = query_schema_consumers(&conn, "proto/user.proto").unwrap();
        assert_eq!(consumers.len(), 2);

        let from = query_generated_from(&conn, "gen/user.pb.go").unwrap();
        assert_eq!(from.len(), 1);
        assert_eq!(from[0].source_path, "proto/user.proto");
    }

    #[test]
    fn test_clear_boundary_edges() {
        let dir = tempdir().unwrap();
        let conn = open_or_create(&dir.path().join("b2.db"), false).unwrap();

        let edges = vec![BoundaryEdge {
            source_path: "a.proto".into(),
            generated_path: "a.pb.go".into(),
            source_package: None,
            generated_package: None,
            kind: "proto".into(),
        }];
        batch_insert_boundary_edges(&conn, &edges).unwrap();
        clear_boundary_edges(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM boundary_edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib db::queries::boundary_tests`
Expected: FAIL — structs and functions not found.

- [ ] **Step 3: Implement structs and functions**

Add to `src/db/queries.rs` (before the `#[cfg(test)] mod tests` block):

```rust
// ── Boundary edge queries ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BoundaryEdge {
    pub source_path: String,
    pub generated_path: String,
    pub source_package: Option<String>,
    pub generated_package: Option<String>,
    pub kind: String,
}

pub fn batch_insert_boundary_edges(conn: &Connection, edges: &[BoundaryEdge]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO boundary_edges \
         (source_path, generated_path, source_package, generated_package, kind) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for e in edges {
        stmt.execute(rusqlite::params![
            e.source_path,
            e.generated_path,
            e.source_package,
            e.generated_package,
            e.kind,
        ])?;
    }
    Ok(())
}

pub fn clear_boundary_edges(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM boundary_edges", [])?;
    Ok(())
}

pub fn query_schema_consumers(conn: &Connection, source_path: &str) -> Result<Vec<BoundaryEdge>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_path, generated_path, source_package, generated_package, kind \
         FROM boundary_edges WHERE source_path = ?1 ORDER BY generated_path",
    )?;
    let rows = stmt.query_map([source_path], |row| {
        Ok(BoundaryEdge {
            source_path: row.get(0)?,
            generated_path: row.get(1)?,
            source_package: row.get(2)?,
            generated_package: row.get(3)?,
            kind: row.get(4)?,
        })
    })?;
    Ok(collect_rows(rows))
}

pub fn query_generated_from(conn: &Connection, generated_path: &str) -> Result<Vec<BoundaryEdge>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source_path, generated_path, source_package, generated_package, kind \
         FROM boundary_edges WHERE generated_path = ?1 ORDER BY source_path",
    )?;
    let rows = stmt.query_map([generated_path], |row| {
        Ok(BoundaryEdge {
            source_path: row.get(0)?,
            generated_path: row.get(1)?,
            source_package: row.get(2)?,
            generated_package: row.get(3)?,
            kind: row.get(4)?,
        })
    })?;
    Ok(collect_rows(rows))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib db::queries::boundary_tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/db/queries.rs
git commit -m "feat: add boundary edge query functions"
```

---

### Task 4: Add boundary detection to the index pipeline

**Files:**
- Modify: `src/index/mod.rs`

This is the core logic: during `phase_index_files`, after the file walk collects `Vec<WalkedFile>`, partition into proto stems and generated stems, match them, scope-filter, and batch insert.

- [ ] **Step 1: Write the boundary detection function**

Add a new function `detect_boundary_edges` in `src/index/mod.rs`:

```rust
use crate::symbols::walker::PROTO_GENERATED_SUFFIXES;

/// Detect proto→generated-code boundary edges from walked files.
///
/// Scans walked files for `.proto` files and files matching known generated
/// suffixes. Matches by stem (filename without extension/suffix), then filters
/// by scope: same package, dependent package (via `dependencies` table), or
/// sibling package (shared parent directory).
fn detect_boundary_edges(
    conn: &Connection,
    files: &[(String, Option<String>, String, u64)], // (path, package, extension, size)
) -> Result<Vec<crate::db::queries::BoundaryEdge>> {
    use std::collections::HashMap;

    // Collect proto stems: stem → Vec<(path, package)>
    let mut proto_map: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    // Collect generated stems: stem → Vec<(path, package, suffix)>
    let mut generated_map: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();

    for (path, package, extension, _size) in files {
        let filename = path.rsplit_once('/').map(|(_, f)| f).unwrap_or(path);

        if extension == "proto" {
            let stem = filename.strip_suffix(".proto").unwrap_or(filename);
            proto_map
                .entry(stem.to_string())
                .or_default()
                .push((path.clone(), package.clone()));
            continue;
        }

        for suffix in PROTO_GENERATED_SUFFIXES {
            if filename.ends_with(suffix) {
                let stem = &filename[..filename.len() - suffix.len()];
                generated_map
                    .entry(stem.to_string())
                    .or_default()
                    .push((path.clone(), package.clone()));
                break; // a file matches at most one suffix
            }
        }
    }

    if proto_map.is_empty() || generated_map.is_empty() {
        return Ok(Vec::new());
    }

    // Load package name→path mapping for sibling-directory comparison
    let pkg_paths: HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT name, path FROM packages")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    // Load dependency edges for scope filtering: set of (dependent, dependency)
    let dep_edges: HashSet<(String, String)> = {
        let mut stmt = conn.prepare("SELECT package, dependency FROM dependencies WHERE is_internal = 1")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut edges = Vec::new();

    for (stem, protos) in &proto_map {
        let gen_files = match generated_map.get(stem) {
            Some(g) => g,
            None => continue,
        };

        for (proto_path, proto_pkg) in protos {
            let proto_pkg_path = proto_pkg.as_deref().and_then(|n| pkg_paths.get(n)).map(|s| s.as_str());
            let proto_parent = proto_pkg_path.and_then(package_parent);

            for (gen_path, gen_pkg) in gen_files {
                let gen_pkg_path = gen_pkg.as_deref().and_then(|n| pkg_paths.get(n)).map(|s| s.as_str());
                if !is_in_scope(proto_pkg.as_deref(), gen_pkg.as_deref(), gen_pkg_path, &proto_parent, &dep_edges) {
                    continue;
                }

                edges.push(crate::db::queries::BoundaryEdge {
                    source_path: proto_path.clone(),
                    generated_path: gen_path.clone(),
                    source_package: proto_pkg.clone(),
                    generated_package: gen_pkg.clone(),
                    kind: "proto".into(),
                });
            }
        }
    }

    Ok(edges)
}

/// Extract the parent directory of a package path for sibling-package matching.
/// "services/auth/proto" → "services/auth", "proto" → None
fn package_parent(pkg_path: &str) -> Option<String> {
    pkg_path.rsplit_once('/').map(|(parent, _)| parent.to_string())
}

/// Check if a generated file is in scope relative to its proto source.
/// Accepts the pair if: same package, generated depends on proto's package,
/// or both packages share a parent directory.
fn is_in_scope(
    proto_pkg: Option<&str>,
    gen_pkg: Option<&str>,
    gen_pkg_path: Option<&str>,
    proto_parent: &Option<String>,
    dep_edges: &HashSet<(String, String)>,
) -> bool {
    match (proto_pkg, gen_pkg) {
        // Both have packages — check relationships
        (Some(pp), Some(gp)) => {
            // Same package
            if pp == gp {
                return true;
            }
            // Generated package depends on proto package
            if dep_edges.contains(&(gp.to_string(), pp.to_string())) {
                return true;
            }
            // Sibling packages (share parent directory path)
            if let Some(proto_par) = proto_parent {
                if let Some(gen_par) = gen_pkg_path.and_then(package_parent) {
                    if *proto_par == gen_par {
                        return true;
                    }
                }
            }
            false
        }
        // Either missing a package — accept (can't scope-filter, better to
        // have a possible false positive than a definite false negative for
        // files not mapped to any package)
        _ => true,
    }
}
```

- [ ] **Step 2: Wire into phase_index_files**

In `src/index/mod.rs`, at the end of `phase_index_files` (after `incremental_upsert_files` and before storing the hash/timestamp), add the boundary detection call:

```rust
    // Detect proto→generated boundary edges from the walked file set.
    // Runs after file upsert so package associations are current.
    crate::db::queries::clear_boundary_edges(conn)?;
    let boundary_edges = detect_boundary_edges(conn, &validated_files)?;
    if !boundary_edges.is_empty() {
        tracing::debug!(edges = boundary_edges.len(), "boundary edges detected");
        crate::db::queries::batch_insert_boundary_edges(conn, &boundary_edges)?;
    }
```

Also add the same block in the early-return paths where the file tree is unchanged (after the `.git/index` mtime check and the file-tree hash check). Since boundary edges depend on file state which hasn't changed, we can skip recomputation — they're already in the DB from the last build that did change.

- [ ] **Step 3: Run cargo check**

Run: `cargo check`
Expected: compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add src/index/mod.rs
git commit -m "feat: add proto boundary detection to index pipeline"
```

---

### Task 5: Add integration test for boundary detection

**Files:**
- Modify: `src/index/mod.rs` (unit test at the bottom)

- [ ] **Step 1: Write the test**

Add a test in the existing `#[cfg(test)]` block of `src/index/mod.rs`:

```rust
#[test]
fn test_detect_boundary_edges_matches_by_stem_and_scope() {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::create_schema_for_test(&conn);

    // Two packages: proto-pkg and go-pkg in the same parent dir
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('proto-pkg', 'services/auth/proto', 'proto')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('go-pkg', 'services/auth/gen', 'go')",
        [],
    ).unwrap();

    let files = vec![
        ("services/auth/proto/user.proto".into(), Some("proto-pkg".into()), "proto".into(), 100u64),
        ("services/auth/gen/user.pb.go".into(), Some("go-pkg".into()), "go".into(), 200),
        ("services/auth/gen/user_pb2.py".into(), Some("go-pkg".into()), "py".into(), 150),
        // Out-of-scope: different parent directory, no dependency
        ("services/billing/gen/user.pb.go".into(), Some("billing-pkg".into()), "go".into(), 200),
    ];

    let edges = detect_boundary_edges(&conn, &files).unwrap();

    assert_eq!(edges.len(), 2, "should match user.pb.go and user_pb2.py in sibling package");
    assert!(edges.iter().all(|e| e.source_path == "services/auth/proto/user.proto"));
    let gen_paths: HashSet<&str> = edges.iter().map(|e| e.generated_path.as_str()).collect();
    assert!(gen_paths.contains("services/auth/gen/user.pb.go"));
    assert!(gen_paths.contains("services/auth/gen/user_pb2.py"));
    // billing should be excluded — different parent, no dep edge
    assert!(!gen_paths.contains("services/billing/gen/user.pb.go"));
}

#[test]
fn test_detect_boundary_edges_dep_scope() {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::create_schema_for_test(&conn);

    conn.execute(
        "INSERT INTO packages (name, path, kind) VALUES ('proto-pkg', 'proto', 'proto'), ('consumer', 'apps/consumer', 'go')",
        [],
    ).unwrap();
    // consumer depends on proto-pkg
    conn.execute(
        "INSERT INTO dependencies (package, dependency, dep_kind, is_internal) VALUES ('consumer', 'proto-pkg', 'runtime', 1)",
        [],
    ).unwrap();

    let files = vec![
        ("proto/api.proto".into(), Some("proto-pkg".into()), "proto".into(), 100u64),
        ("apps/consumer/api.pb.go".into(), Some("consumer".into()), "go".into(), 200),
    ];

    let edges = detect_boundary_edges(&conn, &files).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].generated_package.as_deref(), Some("consumer"));
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib index::tests::test_detect_boundary_edges`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/index/mod.rs
git commit -m "test: add boundary detection unit tests"
```

---

### Task 6: Add schema_consumers and generated_from MCP tools

**Files:**
- Modify: `src/mcp/tools.rs`

- [ ] **Step 1: Add args structs**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SchemaConsumersArgs {
    /// Path to the schema file (e.g. "proto/user.proto")
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GeneratedFromArgs {
    /// Path to the generated file (e.g. "gen/user.pb.go")
    pub path: String,
}
```

- [ ] **Step 2: Add tool handlers**

Add after the `change_impact` tool in the `impl ShireService` block:

```rust
#[tool(description = "Find all files generated from a schema file (e.g. .proto). Returns generated file paths and their packages. Use to understand the blast radius of a schema change.")]
fn schema_consumers(
    &self,
    Parameters(args): Parameters<SchemaConsumersArgs>,
) -> Result<CallToolResult, ErrorData> {
    tracing::debug!(tool = "schema_consumers", path = %args.path);
    self.maybe_rebuild();
    let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
    let rows = queries::query_schema_consumers(&conn, &args.path)
        .map_err(|e| Self::mcp_err(e.to_string()))?;
    let json = serde_json::to_string(&rows)
        .map_err(|e| Self::mcp_err(e.to_string()))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

#[tool(description = "Find the source schema file that generated a given file. Use to trace a generated file (e.g. user.pb.go) back to its source proto.")]
fn generated_from(
    &self,
    Parameters(args): Parameters<GeneratedFromArgs>,
) -> Result<CallToolResult, ErrorData> {
    tracing::debug!(tool = "generated_from", path = %args.path);
    self.maybe_rebuild();
    let conn = self.conn.lock().map_err(|e| Self::mcp_err(e.to_string()))?;
    let rows = queries::query_generated_from(&conn, &args.path)
        .map_err(|e| Self::mcp_err(e.to_string()))?;
    let json = serde_json::to_string(&rows)
        .map_err(|e| Self::mcp_err(e.to_string()))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
```

- [ ] **Step 3: Add tests**

```rust
#[test]
fn test_schema_consumers_empty_db() {
    let svc = make_service_readonly();
    let args = SchemaConsumersArgs { path: "a.proto".into() };
    let r = svc.schema_consumers(Parameters(args)).unwrap();
    let text = match &r.content.first().expect("content").raw {
        RawContent::Text(t) => t.text.clone(),
        _ => panic!("expected text"),
    };
    assert_eq!(text, "[]");
}

#[test]
fn test_generated_from_empty_db() {
    let svc = make_service_readonly();
    let args = GeneratedFromArgs { path: "a.pb.go".into() };
    let r = svc.generated_from(Parameters(args)).unwrap();
    let text = match &r.content.first().expect("content").raw {
        RawContent::Text(t) => t.text.clone(),
        _ => panic!("expected text"),
    };
    assert_eq!(text, "[]");
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib mcp::tools::tests::test_schema_consumers && cargo test --lib mcp::tools::tests::test_generated_from`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: add schema_consumers and generated_from MCP tools"
```

---

### Task 7: Update documentation

**Files:**
- Modify: `docs/src/mcp-tools.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add tool rows to mcp-tools.md**

Add after the `change_impact` row in the tools table:

```markdown
| `schema_consumers` | Find all files generated from a schema file (e.g. `.proto`). Returns generated file paths and their packages. Use to understand the blast radius of a schema change. |
| `generated_from` | Find the source schema file that generated a given file. Use to trace a generated file (e.g. `user.pb.go`) back to its source proto. |
```

- [ ] **Step 2: Update CLAUDE.md**

In the `mcp/` module description, bump tool count from 15 to 17. Add boundary tools to the description:

```
15 tools → 17 tools
Four reference tools → Four reference tools + two boundary tools: `schema_consumers` (proto → generated), `generated_from` (generated → proto)
```

In the `index/` module description, add mention of boundary detection:

```
Detects proto→generated-code boundary edges during file walking using stem matching and package-scope filtering.
```

- [ ] **Step 3: Commit**

```bash
git add docs/src/mcp-tools.md CLAUDE.md
git commit -m "docs: add schema_consumers and generated_from tools"
```

---

### Task 8: Full verification

- [ ] **Step 1: Run all unit tests**

Run: `cargo test --lib`
Expected: all pass (680+)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings

- [ ] **Step 3: Run integration tests**

Run: `cargo test --test integration`
Expected: all pass

- [ ] **Step 4: Verify final diff**

Run: `git diff main --stat`
Expected changes in: `src/symbols/walker.rs`, `src/db/mod.rs`, `src/db/queries.rs`, `src/index/mod.rs`, `src/mcp/tools.rs`, `docs/src/mcp-tools.md`, `CLAUDE.md`
