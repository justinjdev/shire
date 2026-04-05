# Cross-Reference Index Implementation Plan

> **NOTE (historical):** This plan was written before implementation. The final
> shipped design dropped the FTS5 virtual table for `symbol_refs` in favor of
> B-tree indexes only — MCP query tools use exact-name lookups, which don't
> benefit from FTS5 ranking. The FTS5 schema, triggers, and helpers described
> in Task 4 were never merged. Schema changes also landed differently: refs
> store `file_id INTEGER` (FK to `files`), not the original `file_path TEXT`.
> Use this document for historical context only; see
> `docs/src/architecture.md` for the shipped design.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Index symbol references (call sites, type refs, imports, impl clauses) alongside symbol definitions, exposing three new MCP tools (`symbol_references`, `symbol_callers`, `symbol_callees`) for tier 1 languages: Go, Python, Java, TypeScript, JavaScript, Perl, Ruby, Scala.

**Architecture:** Extend existing per-language tree-sitter `.scm` files with `@reference.call`, `@reference.type`, `@reference.import`, `@reference.impl` captures. One parse per file produces both `Vec<SymbolInfo>` and `Vec<ReferenceInfo>`. References persist to a new `symbol_refs` SQLite table with FTS5 index. Incremental rebuild reuses existing file-hash pipeline.

**Tech Stack:** Rust 2024, tree-sitter 0.25, rusqlite (SQLite + FTS5), rmcp (MCP server).

**Spec:** `docs/superpowers/specs/2026-04-04-cross-reference-index-design.md`

---

## Phase 1: Foundation types and extraction infrastructure

### Task 1: Add `ReferenceInfo` and `ReferenceKind` types

**Files:**
- Modify: `src/symbols/mod.rs`

- [ ] **Step 1: Add the types to `src/symbols/mod.rs`**

Insert after the existing `Parameter` struct (around line 58):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ReferenceInfo {
    pub name: String,
    pub kind: ReferenceKind,
    pub file_path: Arc<str>,
    pub line: usize,
    pub enclosing_symbol: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Call,
    Type,
    Import,
    Impl,
}

impl ReferenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReferenceKind::Call => "call",
            ReferenceKind::Type => "type",
            ReferenceKind::Import => "import",
            ReferenceKind::Impl => "impl",
        }
    }
}
```

- [ ] **Step 2: Build to verify compile**

Run: `cargo build 2>&1 | tail -20`
Expected: `warning: struct ... is never used` (dead code warnings expected) — no errors.

- [ ] **Step 3: Commit**

```bash
git add src/symbols/mod.rs
git commit -m "feat(symbols): add ReferenceInfo and ReferenceKind types"
```

---

### Task 2: Extend `LanguageHooks` with reference-extraction fields

**Files:**
- Modify: `src/symbols/hooks/mod.rs`

- [ ] **Step 1: Add fields to the `LanguageHooks` struct**

In `src/symbols/hooks/mod.rs`, inside the `LanguageHooks` struct, add two new fields after `post_process` (keep all existing fields):

```rust
    /// Node kinds that qualify as an enclosing symbol for references.
    /// The extractor walks up from a reference node through ancestors, stopping
    /// at the first node whose kind appears in this list. None means the
    /// language has no references tracked (empty list acceptable too).
    pub enclosing_ancestors: &'static [&'static str],

    /// Identifiers to skip when emitting references (language built-ins,
    /// reserved words that parse as identifiers, etc.).
    pub reference_stoplist: &'static [&'static str],
```

- [ ] **Step 2: Update the `Default` impl**

```rust
impl Default for LanguageHooks {
    fn default() -> Self {
        Self {
            is_visible: None,
            resolve_parent: None,
            build_signature: None,
            extract_parameters: None,
            extract_return_type: None,
            post_process: None,
            enclosing_ancestors: &[],
            reference_stoplist: &[],
        }
    }
}
```

- [ ] **Step 3: Add a shared helper `resolve_enclosing_symbol`**

At the bottom of `src/symbols/hooks/mod.rs`:

```rust
/// Walk up from `node` through ancestors looking for the first node whose kind
/// is listed in `ancestors`. Returns the text of that ancestor's `name` field
/// (or its first `identifier`/`type_identifier` child) as the enclosing symbol
/// name. Returns None if no qualifying named ancestor is found.
pub fn resolve_enclosing_symbol(
    node: &Node,
    source: &str,
    ancestors: &[&str],
) -> Option<String> {
    let mut current = node.parent();
    while let Some(n) = current {
        if ancestors.contains(&n.kind()) {
            // Try the `name` field first (most grammars)
            if let Some(name) = field_text(&n, "name", source) {
                return Some(name.to_string());
            }
            // Fall back to scanning direct children for an identifier-like node
            for i in 0..n.child_count() {
                let child = n.child(i).unwrap();
                match child.kind() {
                    "identifier" | "type_identifier" | "constant" | "simple_identifier" => {
                        if let Some(txt) = node_text(&child, source) {
                            return Some(txt.to_string());
                        }
                    }
                    _ => {}
                }
            }
            // Found an ancestor but couldn't name it — continue walking
        }
        current = n.parent();
    }
    None
}
```

- [ ] **Step 4: Build to verify compile**

Run: `cargo build 2>&1 | tail -20`
Expected: No errors. Per-language hooks files still compile because the new fields have defaults.

- [ ] **Step 5: Commit**

```bash
git add src/symbols/hooks/mod.rs
git commit -m "feat(symbols): extend LanguageHooks for reference extraction"
```

---

### Task 3: Produce references from `query_extract::extract()`

**Files:**
- Modify: `src/symbols/query_extract.rs`
- Modify: `src/symbols/mod.rs`
- Modify: `src/symbols/registry.rs`

- [ ] **Step 1: Update `query_extract::extract()` to return references too**

Replace the function signature and body in `src/symbols/query_extract.rs`:

```rust
use std::sync::Arc;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use super::hooks::{resolve_enclosing_symbol, LanguageHooks};
use super::{ReferenceInfo, ReferenceKind, SymbolInfo, SymbolKind};

fn capture_name_to_kind(name: &str) -> Option<SymbolKind> {
    match name {
        "definition.function" => Some(SymbolKind::Function),
        "definition.method" => Some(SymbolKind::Method),
        "definition.class" => Some(SymbolKind::Class),
        "definition.struct" => Some(SymbolKind::Struct),
        "definition.interface" => Some(SymbolKind::Interface),
        "definition.enum" => Some(SymbolKind::Enum),
        "definition.trait" => Some(SymbolKind::Trait),
        "definition.type" => Some(SymbolKind::Type),
        "definition.constant" => Some(SymbolKind::Constant),
        "definition.module" => Some(SymbolKind::Class),
        _ => None,
    }
}

fn capture_name_to_ref_kind(name: &str) -> Option<ReferenceKind> {
    match name {
        "reference.call" => Some(ReferenceKind::Call),
        "reference.type" => Some(ReferenceKind::Type),
        "reference.import" => Some(ReferenceKind::Import),
        "reference.impl" => Some(ReferenceKind::Impl),
        _ => None,
    }
}

pub fn extract(
    parser: &mut Parser,
    query: &Query,
    source: &str,
    file_path: Arc<str>,
    hooks: &LanguageHooks,
) -> (Vec<SymbolInfo>, Vec<ReferenceInfo>) {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return (Vec::new(), Vec::new()),
    };

    let capture_names = query.capture_names();
    let name_idx = match capture_names.iter().position(|&n| n == "name") {
        Some(i) => i as u32,
        None => return (Vec::new(), Vec::new()),
    };

    let mut cursor = QueryCursor::new();
    let mut symbols = Vec::new();
    let mut references = Vec::new();
    let mut seen_def_ranges = std::collections::HashSet::new();
    let source_bytes = source.as_bytes();

    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
    while let Some(m) = matches.next() {
        let name_capture = m.captures.iter().find(|c| c.index == name_idx);
        let name = match name_capture {
            Some(c) => match c.node.utf8_text(source_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            },
            None => continue,
        };

        // Try definition captures first
        let mut def_kind = None;
        let mut def_node = None;
        let mut ref_kind = None;
        let mut ref_node = None;
        for capture in m.captures.iter() {
            let cname = capture_names[capture.index as usize];
            if def_kind.is_none() {
                if let Some(k) = capture_name_to_kind(cname) {
                    def_kind = Some(k);
                    def_node = Some(capture.node);
                    continue;
                }
            }
            if ref_kind.is_none() {
                if let Some(k) = capture_name_to_ref_kind(cname) {
                    ref_kind = Some(k);
                    ref_node = Some(capture.node);
                }
            }
        }

        // Definition path (unchanged logic)
        if let (Some(kind), Some(node)) = (def_kind, def_node) {
            let range_key = (node.start_byte(), node.end_byte());
            if !seen_def_ranges.insert(range_key) {
                continue;
            }
            if let Some(is_visible) = hooks.is_visible {
                if !is_visible(&node, source) {
                    continue;
                }
            }
            let line = node.start_position().row + 1;
            let parent = hooks.resolve_parent.and_then(|f| f(&node, source));
            let signature = hooks
                .build_signature
                .map(|f| f(&node, source, &name, kind))
                .unwrap_or_else(|| default_signature(&name, kind));
            let parameters = if matches!(kind, SymbolKind::Function | SymbolKind::Method) {
                Some(
                    hooks
                        .extract_parameters
                        .map(|f| f(&node, source))
                        .unwrap_or_default(),
                )
            } else {
                None
            };
            let return_type = if matches!(kind, SymbolKind::Function | SymbolKind::Method) {
                hooks.extract_return_type.and_then(|f| f(&node, source))
            } else {
                None
            };
            let sym = SymbolInfo {
                name: name.clone(),
                kind,
                signature: Some(signature),
                file_path: file_path.clone(),
                line,
                visibility: "public".to_string(),
                parent_symbol: parent,
                return_type,
                parameters,
            };
            let sym = if let Some(post) = hooks.post_process {
                match post(sym, &node, source) {
                    Some(s) => s,
                    None => continue,
                }
            } else {
                sym
            };
            symbols.push(sym);
            continue;
        }

        // Reference path
        if let (Some(kind), Some(node)) = (ref_kind, ref_node) {
            // Skip stoplist names
            if hooks.reference_stoplist.contains(&name.as_str()) {
                continue;
            }
            let line = node.start_position().row + 1;
            let enclosing =
                resolve_enclosing_symbol(&node, source, hooks.enclosing_ancestors);
            references.push(ReferenceInfo {
                name,
                kind,
                file_path: file_path.clone(),
                line,
                enclosing_symbol: enclosing,
            });
        }
    }

    (symbols, references)
}

fn default_signature(name: &str, kind: SymbolKind) -> String {
    let keyword = match kind {
        SymbolKind::Function | SymbolKind::Method => "fn",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Interface => "interface",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Type => "type",
        SymbolKind::Constant => "const",
    };
    format!("{} {}", keyword, name)
}
```

- [ ] **Step 2: Update `registry::extract_file` to return both vecs**

In `src/symbols/registry.rs`, change the signature and return value:

```rust
pub fn extract_file(
    ext: &str,
    source: &str,
    file_path: Arc<str>,
) -> (Vec<SymbolInfo>, Vec<ReferenceInfo>) {
    // Regex-based extractors (no tree-sitter)
    match ext {
        "cob" | "cbl" | "cpy" => {
            return (super::cobol::extract(source, file_path), Vec::new());
        }
        _ => {}
    }

    for entry in registry() {
        if entry.extensions.contains(&ext) {
            let query = entry.query();
            let hooks = (entry.hooks)();
            let language = (entry.ts_language)();
            let mut parser = Parser::new();
            if parser.set_language(&language).is_err() {
                return (Vec::new(), Vec::new());
            }
            return query_extract::extract(&mut parser, query, source, file_path, &hooks);
        }
    }

    (Vec::new(), Vec::new())
}
```

Also update the import line at the top of `registry.rs`:

```rust
use super::{query_extract, ReferenceInfo, SymbolInfo};
```

- [ ] **Step 3: Add a convenience `extract_file` wrapper in `src/symbols/mod.rs`**

Replace the existing `extract_file` function with two functions — a full one and a symbols-only convenience wrapper:

```rust
/// Extract both symbols and references from a single file by extension.
pub fn extract_file_full(
    ext: &str,
    source: &str,
    file_path: Arc<str>,
) -> (Vec<SymbolInfo>, Vec<ReferenceInfo>) {
    registry::extract_file(ext, source, file_path)
}

/// Extract only symbols (backward-compatible convenience wrapper).
pub fn extract_file(ext: &str, source: &str, file_path: Arc<str>) -> Vec<SymbolInfo> {
    registry::extract_file(ext, source, file_path).0
}
```

- [ ] **Step 4: Build to verify all callers still compile**

Run: `cargo build 2>&1 | tail -30`
Expected: No errors. The 122+ `extract_file` call sites in tests continue to work.

- [ ] **Step 5: Run existing tests to confirm no behavior change**

Run: `cargo test --lib symbols:: 2>&1 | tail -20`
Expected: All tests pass — symbol extraction is unchanged.

- [ ] **Step 6: Commit**

```bash
git add src/symbols/query_extract.rs src/symbols/registry.rs src/symbols/mod.rs
git commit -m "feat(symbols): query_extract now returns (symbols, references)"
```

---

## Phase 2: Database schema and persistence

### Task 4: Add `symbol_refs` table, FTS5 index, and triggers

**Files:**
- Modify: `src/db/mod.rs`

- [ ] **Step 1: Add schema to `create_schema()`**

In `src/db/mod.rs`, append the following to the `execute_batch` string inside `create_schema()` (before the closing `",`):

```sql
CREATE TABLE IF NOT EXISTS symbol_refs (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT NOT NULL,
    kind             TEXT NOT NULL,
    file_path        TEXT NOT NULL,
    line             INTEGER NOT NULL,
    package          TEXT,
    enclosing_symbol TEXT
);

CREATE INDEX IF NOT EXISTS idx_refs_name ON symbol_refs(name);
CREATE INDEX IF NOT EXISTS idx_refs_file ON symbol_refs(file_path);
CREATE INDEX IF NOT EXISTS idx_refs_enclosing ON symbol_refs(enclosing_symbol);
CREATE INDEX IF NOT EXISTS idx_refs_package ON symbol_refs(package);

CREATE VIRTUAL TABLE IF NOT EXISTS symbol_refs_fts USING fts5(
    name, kind, enclosing_symbol,
    content='symbol_refs',
    content_rowid='rowid',
    tokenize="unicode61 tokenchars '_'",
    prefix='2,3'
);

CREATE TRIGGER IF NOT EXISTS symbol_refs_ai AFTER INSERT ON symbol_refs BEGIN
    INSERT INTO symbol_refs_fts(rowid, name, kind, enclosing_symbol)
    VALUES (new.rowid, new.name, new.kind, new.enclosing_symbol);
END;

CREATE TRIGGER IF NOT EXISTS symbol_refs_ad AFTER DELETE ON symbol_refs BEGIN
    INSERT INTO symbol_refs_fts(symbol_refs_fts, rowid, name, kind, enclosing_symbol)
    VALUES ('delete', old.rowid, old.name, old.kind, old.enclosing_symbol);
END;

CREATE TRIGGER IF NOT EXISTS symbol_refs_au AFTER UPDATE ON symbol_refs BEGIN
    INSERT INTO symbol_refs_fts(symbol_refs_fts, rowid, name, kind, enclosing_symbol)
    VALUES ('delete', old.rowid, old.name, old.kind, old.enclosing_symbol);
    INSERT INTO symbol_refs_fts(rowid, name, kind, enclosing_symbol)
    VALUES (new.rowid, new.name, new.kind, new.enclosing_symbol);
END;
```

- [ ] **Step 2: Bump the FTS schema version and include new table in migration drops**

Change `FTS_SCHEMA_VERSION` from `"3"` to `"4"` at line 301.

Add the new table/trigger drops to `migrate_fts_if_needed` (the `execute_batch` around line 322):

```sql
DROP TRIGGER IF EXISTS symbol_refs_ai;
DROP TRIGGER IF EXISTS symbol_refs_ad;
DROP TRIGGER IF EXISTS symbol_refs_au;
DROP TABLE IF EXISTS symbol_refs_fts;
```

And add the FTS rebuild for the new table after `create_schema(conn)?;`:

```sql
INSERT INTO symbol_refs_fts(symbol_refs_fts) VALUES('rebuild');
```

(Append it to the existing rebuild batch at line 343–348.)

- [ ] **Step 3: Add drop/recreate trigger helpers**

After `recreate_docs_fts_triggers`, add:

```rust
pub fn drop_symbol_refs_fts_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS symbol_refs_ai;
         DROP TRIGGER IF EXISTS symbol_refs_ad;
         DROP TRIGGER IF EXISTS symbol_refs_au;",
    )?;
    Ok(())
}

pub fn recreate_symbol_refs_fts_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS symbol_refs_ai AFTER INSERT ON symbol_refs BEGIN
            INSERT INTO symbol_refs_fts(rowid, name, kind, enclosing_symbol)
            VALUES (new.rowid, new.name, new.kind, new.enclosing_symbol);
        END;
        CREATE TRIGGER IF NOT EXISTS symbol_refs_ad AFTER DELETE ON symbol_refs BEGIN
            INSERT INTO symbol_refs_fts(symbol_refs_fts, rowid, name, kind, enclosing_symbol)
            VALUES ('delete', old.rowid, old.name, old.kind, old.enclosing_symbol);
        END;
        CREATE TRIGGER IF NOT EXISTS symbol_refs_au AFTER UPDATE ON symbol_refs BEGIN
            INSERT INTO symbol_refs_fts(symbol_refs_fts, rowid, name, kind, enclosing_symbol)
            VALUES ('delete', old.rowid, old.name, old.kind, old.enclosing_symbol);
            INSERT INTO symbol_refs_fts(rowid, name, kind, enclosing_symbol)
            VALUES (new.rowid, new.name, new.kind, new.enclosing_symbol);
        END;",
    )?;
    Ok(())
}
```

- [ ] **Step 4: Write a test verifying schema creation**

Add to an existing db test module (or create `src/db/tests.rs` if none exists). First check for test file:

```bash
ls src/db/
```

If `tests.rs` does not exist, add this test to the bottom of `src/db/mod.rs`:

```rust
#[cfg(test)]
mod schema_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_symbol_refs_schema_created() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_or_create(&path, false).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='symbol_refs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "symbol_refs table must exist");

        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='symbol_refs_fts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1, "symbol_refs_fts virtual table must exist");
    }

    #[test]
    fn test_symbol_refs_insert_and_fts() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_or_create(&path, false).unwrap();

        // need a package row — symbol_refs.package is nullable but let's test NULL package
        conn.execute(
            "INSERT INTO symbol_refs (name, kind, file_path, line, package, enclosing_symbol) \
             VALUES ('parseConfig', 'call', 'src/main.rs', 42, NULL, 'handle_request')",
            [],
        ).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // FTS trigger should have populated symbol_refs_fts
        let fts_hit: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbol_refs_fts WHERE name MATCH 'parseConfig'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts_hit, 1, "FTS index should contain the inserted row");
    }
}
```

- [ ] **Step 5: Run tests to verify schema and FTS triggers work**

Run: `cargo test --lib db::schema_tests 2>&1 | tail -10`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/db/mod.rs
git commit -m "feat(db): add symbol_refs table with FTS5 index"
```

---

### Task 5: Add `batch_insert_references` and delete helpers

**Files:**
- Modify: `src/db/queries.rs`

- [ ] **Step 1: Check existing query patterns in queries.rs**

Run: `grep -n "pub fn batch_insert\|pub fn upsert\|pub fn delete" src/db/queries.rs | head -20`
Expected output: list of existing batch helpers to match the style.

- [ ] **Step 2: Add reference-insert and delete helpers**

At the end of `src/db/queries.rs`, append:

```rust
use crate::symbols::ReferenceInfo;

pub fn batch_insert_references(
    conn: &Connection,
    package: Option<&str>,
    refs: &[ReferenceInfo],
) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO symbol_refs (name, kind, file_path, line, package, enclosing_symbol) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for r in refs {
        stmt.execute(rusqlite::params![
            r.name,
            r.kind.as_str(),
            &*r.file_path,
            r.line as i64,
            package,
            r.enclosing_symbol,
        ])?;
    }
    Ok(())
}

pub fn delete_references_for_file(conn: &Connection, file_path: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM symbol_refs WHERE file_path = ?1",
        [file_path],
    )?;
    Ok(())
}

pub fn delete_references_for_package(conn: &Connection, package: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM symbol_refs WHERE package = ?1",
        [package],
    )?;
    Ok(())
}
```

Confirm the top of `src/db/queries.rs` already imports `use anyhow::Result;` and `use rusqlite::Connection;` — if not, the helpers will need those imports added.

- [ ] **Step 3: Write a unit test**

Append to an existing test module in `src/db/queries.rs` if one exists, else add:

```rust
#[cfg(test)]
mod refs_tests {
    use super::*;
    use crate::db::open_or_create;
    use crate::symbols::{ReferenceInfo, ReferenceKind};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn test_batch_insert_and_delete_by_file() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("t.db");
        let conn = open_or_create(&db_path, false).unwrap();

        let refs = vec![
            ReferenceInfo {
                name: "foo".into(),
                kind: ReferenceKind::Call,
                file_path: Arc::from("a.rs"),
                line: 10,
                enclosing_symbol: Some("bar".into()),
            },
            ReferenceInfo {
                name: "Baz".into(),
                kind: ReferenceKind::Type,
                file_path: Arc::from("a.rs"),
                line: 12,
                enclosing_symbol: None,
            },
            ReferenceInfo {
                name: "quux".into(),
                kind: ReferenceKind::Call,
                file_path: Arc::from("b.rs"),
                line: 3,
                enclosing_symbol: None,
            },
        ];
        batch_insert_references(&conn, Some("mypkg"), &refs).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);

        delete_references_for_file(&conn, "a.rs").unwrap();

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM symbol_refs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, 1, "only b.rs ref should remain");
    }
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test --lib queries::refs_tests 2>&1 | tail -10`
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add src/db/queries.rs
git commit -m "feat(db): batch insert and delete helpers for symbol_refs"
```

---

### Task 6: Wire references into index build pipeline

**Files:**
- Modify: `src/index/mod.rs`

- [ ] **Step 1: Extend `FileResult` to carry references**

Find the `FileResult` struct in `src/index/mod.rs` (grep shows it around line 1238):

Run: `grep -n "struct FileResult" src/index/mod.rs`

Add a `references` field to the struct. Example shape (adapt field names to match the existing struct):

```rust
struct FileResult {
    file_path: String,
    symbols: Option<Vec<symbols::SymbolInfo>>,
    references: Option<Vec<symbols::ReferenceInfo>>,
    // ... existing fields
}
```

- [ ] **Step 2: Switch extraction call sites to `extract_file_full`**

There are ~2 spots where `symbols::extract_file(ext, &source, file_path_arc)` is called (lines ~978 and ~1236 per earlier grep). Replace each:

```rust
// before
symbols::extract_file(ext, &source, file_path_arc)

// after
symbols::extract_file_full(ext, &source, file_path_arc)
```

And capture both return values. Where the old `.unwrap_or_default()` returns `Vec<SymbolInfo>`, now use a tuple default:

```rust
// Example, preserve surrounding context:
let (syms, refs) = std::fs::read_to_string(path)
    .ok()
    .map(|source| {
        let file_path_arc: Arc<str> = Arc::from(relative_path.as_str());
        symbols::extract_file_full(ext, &source, file_path_arc)
    })
    .unwrap_or_else(|| (Vec::new(), Vec::new()));
```

The same shape applies to both call sites. Store both into the `FileResult`.

- [ ] **Step 3: Update `upsert_symbols_for_file` to also upsert references**

Rename the function and extend its body. Replace:

```rust
fn upsert_symbols_for_file(
    conn: &Connection,
    package: &str,
    file_path: &str,
    syms: &[symbols::SymbolInfo],
) -> Result<()> {
    conn.execute(
        "DELETE FROM symbols WHERE package = ?1 AND file_path = ?2",
        rusqlite::params![package, file_path],
    )?;
    batch_insert_symbols(conn, package, syms)?;
    Ok(())
}
```

with:

```rust
fn upsert_symbols_and_refs_for_file(
    conn: &Connection,
    package: &str,
    file_path: &str,
    syms: &[symbols::SymbolInfo],
    refs: &[symbols::ReferenceInfo],
) -> Result<()> {
    conn.execute(
        "DELETE FROM symbols WHERE package = ?1 AND file_path = ?2",
        rusqlite::params![package, file_path],
    )?;
    batch_insert_symbols(conn, package, syms)?;

    conn.execute(
        "DELETE FROM symbol_refs WHERE file_path = ?1",
        rusqlite::params![file_path],
    )?;
    crate::db::queries::batch_insert_references(conn, Some(package), refs)?;
    Ok(())
}
```

- [ ] **Step 4: Update the caller of that function**

Find the caller (around line 1316–1318):

```rust
if let Some(syms) = &fr.symbols {
    upsert_symbols_for_file(conn, pkg_name, &fr.file_path, syms)?;
    had_changes = true;
}
```

Replace with:

```rust
if let Some(syms) = &fr.symbols {
    let empty_refs = Vec::new();
    let refs = fr.references.as_ref().unwrap_or(&empty_refs);
    upsert_symbols_and_refs_for_file(conn, pkg_name, &fr.file_path, syms, refs)?;
    had_changes = true;
}
```

- [ ] **Step 5: Propagate deleted-file cleanup to refs**

Find `DELETE FROM symbols WHERE package = ?1 AND file_path = ?2` in the deletion loop (around line 1298):

```rust
for del_path in deleted_files {
    conn.execute(
        "DELETE FROM symbols WHERE package = ?1 AND file_path = ?2",
        rusqlite::params![pkg_name, del_path],
    )?;
}
```

Add a matching references delete inside the same loop:

```rust
for del_path in deleted_files {
    conn.execute(
        "DELETE FROM symbols WHERE package = ?1 AND file_path = ?2",
        rusqlite::params![pkg_name, del_path],
    )?;
    conn.execute(
        "DELETE FROM symbol_refs WHERE file_path = ?1",
        rusqlite::params![del_path],
    )?;
}
```

- [ ] **Step 6: Propagate full-rebuild cleanup to refs**

Find `conn.execute("DELETE FROM symbols", [])?;` in the rebuild path (around line 1980) and add a sibling:

```rust
conn.execute("DELETE FROM symbols", [])?;
conn.execute("DELETE FROM symbol_refs", [])?;
```

And the package-level deletions — add a `DELETE FROM symbol_refs WHERE package = ?1` next to each `DELETE FROM symbols WHERE package = ?1` (there are several; grep for them).

Run: `grep -n "DELETE FROM symbols" src/index/mod.rs`

For each match, add an adjacent `DELETE FROM symbol_refs` using the same filter column (whether it's `package = ?1` or `package IN (...)` subquery — mirror it exactly).

- [ ] **Step 7: Build and run all tests**

Run: `cargo build 2>&1 | tail -30`
Expected: No errors.

Run: `cargo test --lib 2>&1 | tail -20`
Expected: All existing tests still pass. No reference-extraction tests yet, so no assertions about refs — but symbol tests must be green.

- [ ] **Step 8: Commit**

```bash
git add src/index/mod.rs
git commit -m "feat(index): persist references alongside symbols per file"
```

---

## Phase 3: Per-language reference extraction

Each language follows the same TDD loop: write a failing test that asserts references are extracted, update the `.scm` file and hooks, then verify pass. Stop using `extract_file` (symbols-only) and use `extract_file_full` for reference assertions.

### Task 7: Go references (call, type, import)

**Files:**
- Modify: `src/symbols/queries/go.scm`
- Modify: `src/symbols/hooks/go.rs`
- Modify: `src/symbols/tests.rs`

- [ ] **Step 1: Read the existing go.scm and hooks**

Run: `cat src/symbols/queries/go.scm`

Run: `head -20 src/symbols/hooks/go.rs`

- [ ] **Step 2: Write a failing test**

Append to `src/symbols/tests.rs`, in the Go section (after the last Go test, before the Rust section):

```rust
// References

#[test]
fn test_go_call_references() {
    let source = r#"package main

import "fmt"

func handleRequest(req *Request) error {
    cfg := parseConfig(req)
    fmt.Println(cfg)
    return validate(cfg)
}

func parseConfig(r *Request) Config { return Config{} }
func validate(c Config) error { return nil }
"#;
    let (_syms, refs) = extract_file_full("go", source, Arc::from("main.go"));

    let call_refs: Vec<&ReferenceInfo> =
        refs.iter().filter(|r| r.kind == ReferenceKind::Call).collect();
    let names: Vec<&str> = call_refs.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"parseConfig"), "expected call-ref to parseConfig, got {:?}", names);
    assert!(names.contains(&"validate"), "expected call-ref to validate, got {:?}", names);

    let parse_ref = call_refs.iter().find(|r| r.name == "parseConfig").unwrap();
    assert_eq!(parse_ref.enclosing_symbol.as_deref(), Some("handleRequest"));
}

#[test]
fn test_go_type_references() {
    let source = r#"package main

type Config struct { Key string }

func parse(r *Request) Config { return Config{} }
"#;
    let (_syms, refs) = extract_file_full("go", source, Arc::from("main.go"));
    let type_refs: Vec<&ReferenceInfo> =
        refs.iter().filter(|r| r.kind == ReferenceKind::Type).collect();
    let names: Vec<&str> = type_refs.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"Request"), "expected type-ref Request, got {:?}", names);
    assert!(names.contains(&"Config"), "expected type-ref Config, got {:?}", names);
}

#[test]
fn test_go_import_references() {
    let source = r#"package main

import (
    "fmt"
    "strings"
)
"#;
    let (_syms, refs) = extract_file_full("go", source, Arc::from("main.go"));
    let import_refs: Vec<&ReferenceInfo> =
        refs.iter().filter(|r| r.kind == ReferenceKind::Import).collect();
    let names: Vec<&str> = import_refs.iter().map(|r| r.name.as_str()).collect();
    // Go imports are strings like "fmt" — strip quotes in your capture mapping,
    // or accept them as-is. Accept either form here.
    assert!(
        names.iter().any(|n| n == &"fmt" || n == &"\"fmt\""),
        "expected fmt import, got {:?}",
        names
    );
}
```

Also add an import at the top of `src/symbols/tests.rs` if not already present:

```rust
use super::ReferenceInfo;
use super::ReferenceKind;
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib symbols::tests::test_go_call_references 2>&1 | tail -15`
Expected: FAIL — `refs` is empty, assertion fires.

- [ ] **Step 4: Update `src/symbols/queries/go.scm`**

Append to the existing go.scm:

```scheme
; Reference: function/method calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (selector_expression
    field: (field_identifier) @name)) @reference.call

; Reference: type usage (parameters, return types, struct fields)
(type_identifier) @name @reference.type

; Reference: imports (the import path string)
(import_spec
  path: (interpreted_string_literal) @name) @reference.import
```

- [ ] **Step 5: Update `src/symbols/hooks/go.rs` with enclosing_ancestors and stoplist**

Find the `pub fn hooks() -> LanguageHooks { ... }` in `src/symbols/hooks/go.rs`. Add:

```rust
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        // ... existing fields unchanged ...
        enclosing_ancestors: &[
            "function_declaration",
            "method_declaration",
        ],
        reference_stoplist: &[
            "true", "false", "nil", "iota",
            "make", "new", "len", "cap", "append", "copy", "delete",
            "print", "println", "panic", "recover",
            "int", "int32", "int64", "uint", "uint32", "uint64",
            "string", "bool", "byte", "rune", "float32", "float64",
            "error", "any",
        ],
        ..Default::default()  // if you've been using explicit listing, just add the two fields above
    }
}
```

Important: if the existing hooks function constructs `LanguageHooks { ... }` by listing every field, add the two new fields explicitly. If it uses `..Default::default()`, simply add the two fields before the `..`.

- [ ] **Step 6: Handle quoted import paths**

Go's `interpreted_string_literal` node text includes the surrounding quotes (e.g., `"fmt"`). Add a post-extraction trim in `query_extract.rs`.

Option A (simplest): add a `strip_quotes` pass inside `query_extract::extract()` right before pushing the `ReferenceInfo` for `Import` kind only:

```rust
// Before: references.push(ReferenceInfo { name, kind, ... })
let trimmed_name = if matches!(kind, ReferenceKind::Import) {
    name.trim_matches(|c: char| c == '"' || c == '\'' || c == '`').to_string()
} else {
    name
};
references.push(ReferenceInfo {
    name: trimmed_name,
    kind,
    // ...
});
```

- [ ] **Step 7: Build**

Run: `cargo build 2>&1 | tail -10`
Expected: No errors.

- [ ] **Step 8: Run tests**

Run: `cargo test --lib symbols::tests::test_go 2>&1 | tail -30`
Expected: All Go tests pass (existing symbol tests + 3 new reference tests).

If assertions fail, inspect actual captures by adding `dbg!(&refs);` in the test temporarily.

- [ ] **Step 9: Commit**

```bash
git add src/symbols/queries/go.scm src/symbols/hooks/go.rs src/symbols/tests.rs src/symbols/query_extract.rs
git commit -m "feat(symbols): extract call/type/import refs for Go"
```

---

### Task 8: Python references (call, type, import, impl)

**Files:**
- Modify: `src/symbols/queries/python.scm`
- Modify: `src/symbols/hooks/python.rs`
- Modify: `src/symbols/tests.rs`

- [ ] **Step 1: Write the failing tests**

Append to `src/symbols/tests.rs` (in or after the Python section):

```rust
#[test]
fn test_python_call_references() {
    let source = r#"import json

def load(path: str) -> dict:
    raw = open(path).read()
    return json.loads(raw)

def save(path: str, data: dict):
    with open(path, 'w') as f:
        f.write(json.dumps(data))
"#;
    let (_syms, refs) = extract_file_full("py", source, Arc::from("io.py"));
    let call_refs: Vec<&ReferenceInfo> =
        refs.iter().filter(|r| r.kind == ReferenceKind::Call).collect();
    let names: Vec<&str> = call_refs.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"loads"), "expected call json.loads -> loads, got {:?}", names);
    assert!(names.contains(&"dumps"));
    assert!(names.contains(&"read"));

    // open() is in the stoplist (builtin), should not appear
    assert!(!names.contains(&"open"), "open() is builtin, should be in stoplist");

    let loads_ref = call_refs.iter().find(|r| r.name == "loads").unwrap();
    assert_eq!(loads_ref.enclosing_symbol.as_deref(), Some("load"));
}

#[test]
fn test_python_import_references() {
    let source = r#"import json
from typing import List, Dict
from os.path import join
"#;
    let (_syms, refs) = extract_file_full("py", source, Arc::from("imports.py"));
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"json"));
    assert!(names.contains(&"List"));
    assert!(names.contains(&"Dict"));
    assert!(names.contains(&"join"));
}

#[test]
fn test_python_impl_references() {
    let source = r#"class Base:
    pass

class Derived(Base):
    pass

class Multi(Base, Mixin):
    pass
"#;
    let (_syms, refs) = extract_file_full("py", source, Arc::from("cls.py"));
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"Base"), "expected Base as impl ref, got {:?}", names);
    assert!(names.contains(&"Mixin"), "expected Mixin as impl ref, got {:?}", names);
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test --lib symbols::tests::test_python_call_references 2>&1 | tail -15`
Expected: FAIL.

- [ ] **Step 3: Update `src/symbols/queries/python.scm`**

Append:

```scheme
; Reference: function/method calls
(call
  function: (identifier) @name) @reference.call

(call
  function: (attribute
    attribute: (identifier) @name)) @reference.call

; Reference: type annotations and generic args
(type
  (identifier) @name) @reference.type

; Reference: imports
(import_statement
  name: (dotted_name (identifier) @name)) @reference.import

(import_from_statement
  name: (dotted_name (identifier) @name)) @reference.import

; Reference: superclasses (impl)
(class_definition
  superclasses: (argument_list
    (identifier) @name @reference.impl))
```

- [ ] **Step 4: Update `src/symbols/hooks/python.rs`**

Add these two fields to the hooks:

```rust
enclosing_ancestors: &["function_definition", "class_definition"],
reference_stoplist: &[
    "True", "False", "None", "self", "cls",
    "print", "open", "len", "range", "enumerate", "zip", "map", "filter",
    "str", "int", "float", "bool", "list", "dict", "tuple", "set",
    "type", "isinstance", "issubclass", "hasattr", "getattr", "setattr",
    "Exception", "ValueError", "TypeError", "KeyError",
],
```

- [ ] **Step 5: Build and run Python reference tests**

Run: `cargo test --lib symbols::tests::test_python 2>&1 | tail -30`
Expected: all Python tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/symbols/queries/python.scm src/symbols/hooks/python.rs src/symbols/tests.rs
git commit -m "feat(symbols): extract call/type/import/impl refs for Python"
```

---

### Task 9: Java references (call, type, import, impl)

**Files:**
- Modify: `src/symbols/queries/java.scm`
- Modify: `src/symbols/hooks/java.rs`
- Modify: `src/symbols/tests.rs`

- [ ] **Step 1: Write failing tests**

Append to tests.rs:

```rust
#[test]
fn test_java_call_references() {
    let source = r#"package com.example;

import java.util.List;

public class UserService {
    private Database db;

    public User fetchUser(String id) {
        return db.lookup(id);
    }

    public void saveUser(User u) {
        validate(u);
        db.insert(u);
    }

    private void validate(User u) {}
}
"#;
    let (_syms, refs) = extract_file_full("java", source, Arc::from("UserService.java"));
    let call_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert!(call_names.contains(&"lookup"));
    assert!(call_names.contains(&"validate"));
    assert!(call_names.contains(&"insert"));

    let lookup_ref = refs.iter().find(|r| r.name == "lookup" && r.kind == ReferenceKind::Call).unwrap();
    assert_eq!(lookup_ref.enclosing_symbol.as_deref(), Some("fetchUser"));
}

#[test]
fn test_java_impl_references() {
    let source = r#"package com.example;

public class ConcreteService extends BaseService implements Cacheable, Auditable {
}
"#;
    let (_syms, refs) = extract_file_full("java", source, Arc::from("CS.java"));
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"BaseService"), "expected superclass, got {:?}", impl_names);
    assert!(impl_names.contains(&"Cacheable"));
    assert!(impl_names.contains(&"Auditable"));
}

#[test]
fn test_java_import_references() {
    let source = r#"package com.example;

import java.util.List;
import java.util.Map;
"#;
    let (_syms, refs) = extract_file_full("java", source, Arc::from("X.java"));
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    // Java imports are fully qualified; the last segment is what matters for name matching.
    assert!(imp_names.iter().any(|n| n.contains("List")));
    assert!(imp_names.iter().any(|n| n.contains("Map")));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib symbols::tests::test_java_call_references 2>&1 | tail -10`
Expected: FAIL.

- [ ] **Step 3: Update `src/symbols/queries/java.scm`**

Append:

```scheme
; Reference: method calls
(method_invocation
  name: (identifier) @name) @reference.call

; Reference: type usages (parameters, field types, return types)
(type_identifier) @name @reference.type

; Reference: imports
(import_declaration
  (scoped_identifier) @name) @reference.import

(import_declaration
  (identifier) @name) @reference.import

; Reference: superclass
(class_declaration
  (superclass
    (type_identifier) @name @reference.impl))

; Reference: implemented interfaces
(class_declaration
  (super_interfaces
    (type_list
      (type_identifier) @name @reference.impl)))

; Reference: extended interfaces (interface X extends Y)
(interface_declaration
  (extends_interfaces
    (type_list
      (type_identifier) @name @reference.impl)))
```

- [ ] **Step 4: Update `src/symbols/hooks/java.rs`**

Add:

```rust
enclosing_ancestors: &[
    "method_declaration",
    "constructor_declaration",
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
],
reference_stoplist: &[
    "true", "false", "null", "this", "super",
    "String", "Integer", "Long", "Boolean", "Double", "Float", "Object",
    "void", "int", "long", "boolean", "double", "float", "byte", "char", "short",
    "System", "Math",
],
```

- [ ] **Step 5: Build and run Java tests**

Run: `cargo test --lib symbols::tests::test_java 2>&1 | tail -30`
Expected: all Java tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/symbols/queries/java.scm src/symbols/hooks/java.rs src/symbols/tests.rs
git commit -m "feat(symbols): extract call/type/import/impl refs for Java"
```

---

### Task 10: TypeScript references (call, type, import, impl)

**Files:**
- Modify: `src/symbols/queries/typescript.scm`
- Modify: `src/symbols/hooks/typescript.rs`
- Modify: `src/symbols/tests.rs`

- [ ] **Step 1: Write failing tests**

Append to tests.rs:

```rust
#[test]
fn test_typescript_call_references() {
    let source = r#"import { parseConfig } from './config';

export function handle(req: Request): Response {
    const cfg = parseConfig(req.body);
    return buildResponse(cfg);
}

function buildResponse(cfg: Config): Response {
    return new Response();
}
"#;
    let (_syms, refs) = extract_file_full("ts", source, Arc::from("handler.ts"));
    let call_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert!(call_names.contains(&"parseConfig"));
    assert!(call_names.contains(&"buildResponse"));
}

#[test]
fn test_typescript_type_references() {
    let source = r#"interface Config {
    key: string;
}

function handle(req: Request): Response {
    return new Response();
}
"#;
    let (_syms, refs) = extract_file_full("ts", source, Arc::from("h.ts"));
    let type_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Type)
        .map(|r| r.name.as_str())
        .collect();
    assert!(type_names.contains(&"Request"));
    assert!(type_names.contains(&"Response"));
}

#[test]
fn test_typescript_impl_references() {
    let source = r#"interface Service {}
interface Auditable {}
class Base {}

class MyService extends Base implements Service, Auditable {
}
"#;
    let (_syms, refs) = extract_file_full("ts", source, Arc::from("svc.ts"));
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"Base"));
    assert!(impl_names.contains(&"Service"));
    assert!(impl_names.contains(&"Auditable"));
}

#[test]
fn test_typescript_import_references() {
    let source = r#"import { parseConfig, Config } from './config';
import { handler } from './handler';
"#;
    let (_syms, refs) = extract_file_full("ts", source, Arc::from("i.ts"));
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    assert!(imp_names.contains(&"parseConfig"));
    assert!(imp_names.contains(&"Config"));
    assert!(imp_names.contains(&"handler"));
}
```

- [ ] **Step 2: Verify failures**

Run: `cargo test --lib symbols::tests::test_typescript_call 2>&1 | tail -10`
Expected: FAIL.

- [ ] **Step 3: Update `src/symbols/queries/typescript.scm`**

Append:

```scheme
; Reference: calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    property: (property_identifier) @name)) @reference.call

; Reference: type usages
(type_identifier) @name @reference.type
(type_annotation
  (type_identifier) @name @reference.type)
(generic_type
  name: (type_identifier) @name @reference.type)

; Reference: imports (named imports)
(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @name @reference.import))))

; Reference: superclass (extends on class)
(class_declaration
  (class_heritage
    (extends_clause
      value: (identifier) @name @reference.impl)))

; Reference: implemented interfaces
(class_declaration
  (class_heritage
    (implements_clause
      (type_identifier) @name @reference.impl)))
```

- [ ] **Step 4: Update `src/symbols/hooks/typescript.rs`**

Add:

```rust
enclosing_ancestors: &[
    "function_declaration",
    "method_definition",
    "class_declaration",
    "interface_declaration",
    "function_expression",
    "method_signature",
],
reference_stoplist: &[
    "true", "false", "null", "undefined", "this", "super",
    "console", "window", "document",
    "string", "number", "boolean", "any", "unknown", "never", "void",
    "String", "Number", "Boolean", "Object", "Array",
    "Promise", "Error",
],
```

Note: `arrow_function` is intentionally NOT listed — anonymous arrows are skipped to find a named ancestor.

- [ ] **Step 5: Build and run**

Run: `cargo test --lib symbols::tests::test_typescript 2>&1 | tail -30`
Expected: all TS tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/symbols/queries/typescript.scm src/symbols/hooks/typescript.rs src/symbols/tests.rs
git commit -m "feat(symbols): extract call/type/import/impl refs for TypeScript"
```

---

### Task 11: JavaScript references (call, import, impl)

**Files:**
- Modify: `src/symbols/queries/javascript.scm`
- Modify: `src/symbols/tests.rs`

JavaScript shares the `typescript` hooks (per `registry.rs` line 70), so no hooks file changes needed. JS has no static types, so no `@reference.type` captures.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_javascript_call_references() {
    let source = r#"import { parseConfig } from './config.js';

export function handle(req) {
    const cfg = parseConfig(req.body);
    return buildResponse(cfg);
}

function buildResponse(cfg) {
    return {};
}
"#;
    let (_syms, refs) = extract_file_full("js", source, Arc::from("h.js"));
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"parseConfig"));
    assert!(names.contains(&"buildResponse"));
}

#[test]
fn test_javascript_impl_references() {
    let source = r#"class Base {}

class Derived extends Base {}
"#;
    let (_syms, refs) = extract_file_full("js", source, Arc::from("c.js"));
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"Base"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --lib symbols::tests::test_javascript 2>&1 | tail -10`
Expected: FAIL.

- [ ] **Step 3: Update `src/symbols/queries/javascript.scm`**

Append (omit the type captures — JS has no static types):

```scheme
; Reference: calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    property: (property_identifier) @name)) @reference.call

; Reference: imports (named imports)
(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @name @reference.import))))

; Reference: superclass
(class_declaration
  (class_heritage
    (identifier) @name @reference.impl))
```

- [ ] **Step 4: Build and run**

Run: `cargo test --lib symbols::tests::test_javascript 2>&1 | tail -20`
Expected: all JS tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/symbols/queries/javascript.scm src/symbols/tests.rs
git commit -m "feat(symbols): extract call/import/impl refs for JavaScript"
```

---

### Task 12: Ruby references (call, type, import, impl)

**Files:**
- Modify: `src/symbols/queries/ruby.scm`
- Modify: `src/symbols/hooks/ruby.rs`
- Modify: `src/symbols/tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_ruby_call_references() {
    let source = r#"require 'json'

class Loader
  def load(path)
    raw = File.read(path)
    JSON.parse(raw)
  end

  def save(path, data)
    File.write(path, JSON.dump(data))
  end
end
"#;
    let (_syms, refs) = extract_file_full("rb", source, Arc::from("loader.rb"));
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"read"));
    assert!(names.contains(&"parse"));
    assert!(names.contains(&"write"));
    assert!(names.contains(&"dump"));

    let parse_ref = refs.iter().find(|r| r.name == "parse" && r.kind == ReferenceKind::Call).unwrap();
    assert_eq!(parse_ref.enclosing_symbol.as_deref(), Some("load"));
}

#[test]
fn test_ruby_impl_references() {
    let source = r#"class Derived < Base
  include Comparable
  include Enumerable
  extend ModuleMethods
end
"#;
    let (_syms, refs) = extract_file_full("rb", source, Arc::from("d.rb"));
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"Base"), "expected superclass Base, got {:?}", impl_names);
    assert!(impl_names.contains(&"Comparable"));
    assert!(impl_names.contains(&"Enumerable"));
    assert!(impl_names.contains(&"ModuleMethods"));
}

#[test]
fn test_ruby_require_references() {
    let source = r#"require 'json'
require_relative './util'
"#;
    let (_syms, refs) = extract_file_full("rb", source, Arc::from("m.rb"));
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    assert!(imp_names.iter().any(|n| n.contains("json")));
    assert!(imp_names.iter().any(|n| n.contains("util")));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --lib symbols::tests::test_ruby_call_references 2>&1 | tail -15`
Expected: FAIL.

- [ ] **Step 3: Update `src/symbols/queries/ruby.scm`**

Append:

```scheme
; Reference: method calls
(call
  method: (identifier) @name) @reference.call

(call
  method: (constant) @name) @reference.call

; Reference: constant references (type-like)
(constant) @name @reference.type

; Reference: require / require_relative
(call
  method: (identifier) @method_name
  arguments: (argument_list (string (string_content) @name)))
  (#any-of? @method_name "require" "require_relative" "load")
  @reference.import

; Reference: include/prepend/extend (mixins)
(call
  method: (identifier) @method_name
  arguments: (argument_list (constant) @name @reference.impl))
  (#any-of? @method_name "include" "prepend" "extend")

; Reference: superclass in class definition
(class
  superclass: (superclass
    (constant) @name @reference.impl))
```

Note on duplication: constants appearing as `@reference.type` will also match as `@reference.impl` when they're the argument of include/prepend/extend. That's fine — they're two different kinds carrying slightly different meaning.

- [ ] **Step 4: Update `src/symbols/hooks/ruby.rs`**

Add:

```rust
enclosing_ancestors: &[
    "method",
    "singleton_method",
    "class",
    "module",
],
reference_stoplist: &[
    "true", "false", "nil", "self",
    "puts", "print", "p", "pp",
    "String", "Integer", "Float", "Array", "Hash", "Symbol", "Nil",
    "Object", "Class", "Module",
],
```

- [ ] **Step 5: Build and run**

Run: `cargo test --lib symbols::tests::test_ruby 2>&1 | tail -30`
Expected: all Ruby tests pass.

If the tree-sitter-ruby grammar uses slightly different node names, grep for them in existing `src/symbols/queries/ruby.scm` and fix.

- [ ] **Step 6: Commit**

```bash
git add src/symbols/queries/ruby.scm src/symbols/hooks/ruby.rs src/symbols/tests.rs
git commit -m "feat(symbols): extract call/type/import/impl refs for Ruby"
```

---

### Task 13: Scala references (call, type, import, impl)

**Files:**
- Modify: `src/symbols/queries/scala.scm`
- Modify: `src/symbols/hooks/scala.rs`
- Modify: `src/symbols/tests.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn test_scala_call_references() {
    let source = r#"package com.example

object Service {
  def process(req: Request): Response = {
    val cfg = parseConfig(req.body)
    buildResponse(cfg)
  }

  def parseConfig(body: String): Config = Config(body)
  def buildResponse(cfg: Config): Response = Response(cfg.key)
}
"#;
    let (_syms, refs) = extract_file_full("scala", source, Arc::from("svc.scala"));
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"parseConfig"));
    assert!(names.contains(&"buildResponse"));
}

#[test]
fn test_scala_impl_references() {
    let source = r#"trait Service
trait Cacheable
class Base
class MyService extends Base with Service with Cacheable
"#;
    let (_syms, refs) = extract_file_full("scala", source, Arc::from("s.scala"));
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"Base"));
    assert!(impl_names.contains(&"Service"));
    assert!(impl_names.contains(&"Cacheable"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --lib symbols::tests::test_scala_call 2>&1 | tail -10`
Expected: FAIL.

- [ ] **Step 3: Update `src/symbols/queries/scala.scm`**

Append:

```scheme
; Reference: calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (field_expression
    field: (identifier) @name)) @reference.call

; Reference: type usages
(type_identifier) @name @reference.type

; Reference: imports
(import_declaration
  path: (stable_identifier
    (identifier) @name @reference.import))

; Reference: extends clause (superclass + traits via "with")
(extends_clause
  type: (type_identifier) @name @reference.impl)

(extends_clause
  type: (generic_type
    (type_identifier) @name @reference.impl))
```

- [ ] **Step 4: Update `src/symbols/hooks/scala.rs`**

Add:

```rust
enclosing_ancestors: &[
    "function_definition",
    "class_definition",
    "object_definition",
    "trait_definition",
],
reference_stoplist: &[
    "true", "false", "null", "this", "super",
    "Int", "Long", "Short", "Byte", "Float", "Double", "Boolean", "Char", "String", "Unit",
    "Some", "None", "Option", "List", "Seq", "Map", "Set", "Array",
    "println", "print",
],
```

- [ ] **Step 5: Build and run**

Run: `cargo test --lib symbols::tests::test_scala 2>&1 | tail -30`
Expected: all Scala tests pass. Adjust node names if the grammar differs.

- [ ] **Step 6: Commit**

```bash
git add src/symbols/queries/scala.scm src/symbols/hooks/scala.rs src/symbols/tests.rs
git commit -m "feat(symbols): extract call/type/import/impl refs for Scala"
```

---

### Task 14: Perl references (call, import)

**Files:**
- Modify: `src/symbols/queries/perl.scm`
- Modify: `src/symbols/hooks/perl.rs`
- Modify: `src/symbols/tests.rs`

Perl has no static types and no direct syntactic impl form — skip `@reference.type` and `@reference.impl`.

- [ ] **Step 1: Check the Perl grammar**

Run: `cat src/symbols/queries/perl.scm`

The ts-parser-perl grammar has idiosyncratic node names — inspect what's already there first.

- [ ] **Step 2: Write failing tests**

```rust
#[test]
fn test_perl_call_references() {
    let source = r#"package My::Service;

use strict;

sub load {
    my ($path) = @_;
    my $raw = read_file($path);
    return parse_raw($raw);
}

sub parse_raw { my ($r) = @_; return {}; }

1;
"#;
    let (_syms, refs) = extract_file_full("pm", source, Arc::from("Service.pm"));
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"parse_raw"));
}

#[test]
fn test_perl_use_references() {
    let source = r#"package Main;
use strict;
use My::Utils;
use JSON::PP;
1;
"#;
    let (_syms, refs) = extract_file_full("pm", source, Arc::from("Main.pm"));
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    // At minimum, capture the module names (possibly including the `::` path)
    assert!(imp_names.iter().any(|n| n.contains("Utils")));
    assert!(imp_names.iter().any(|n| n.contains("JSON")));
    // "strict" is in the stoplist — skip
}
```

- [ ] **Step 3: Verify failure**

Run: `cargo test --lib symbols::tests::test_perl_call 2>&1 | tail -10`
Expected: FAIL.

- [ ] **Step 4: Update `src/symbols/queries/perl.scm`**

Examine existing captures and add ref patterns. ts-parser-perl exposes nodes like `function_call_expression`, `method_call_expression`, and `use_no_statement`. The exact grammar may differ — inspect with a quick helper. If needed, add:

```scheme
; Reference: subroutine calls
(function_call_expression
  (bareword) @name) @reference.call

; Reference: use/require (imports)
(use_no_statement
  (package) @name @reference.import)
```

If the grammar's actual node names differ, debug by running the parser on the test source and inspecting the tree. One way: add a temporary test that prints the tree, or use `tree-sitter parse` CLI if installed.

- [ ] **Step 5: Update `src/symbols/hooks/perl.rs`**

Add:

```rust
enclosing_ancestors: &[
    "subroutine_declaration_statement",
    "package_statement",
],
reference_stoplist: &[
    "strict", "warnings", "utf8", "feature", "parent", "base",
    "print", "say", "die", "warn", "use", "require",
    "my", "our", "local", "sub", "return", "if", "unless",
    "undef", "defined",
],
```

- [ ] **Step 6: Build and run**

Run: `cargo test --lib symbols::tests::test_perl 2>&1 | tail -30`
Expected: all Perl tests pass.

If `function_call_expression` is not the right node name (Perl grammars vary), grep `tree_sitter_perl` or inspect an existing test fixture by printing `tree.root_node().to_sexp()` temporarily.

- [ ] **Step 7: Commit**

```bash
git add src/symbols/queries/perl.scm src/symbols/hooks/perl.rs src/symbols/tests.rs
git commit -m "feat(symbols): extract call/import refs for Perl"
```

---

## Phase 4: MCP tools

### Task 15: `symbol_references` MCP tool

**Files:**
- Modify: `src/mcp/tools.rs`
- Modify: `src/db/queries.rs`

- [ ] **Step 1: Add the DB query function**

Append to `src/db/queries.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReferenceRow {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: i64,
    pub package: Option<String>,
    pub enclosing_symbol: Option<String>,
}

pub fn query_symbol_references(
    conn: &Connection,
    name: &str,
    kind: Option<&str>,
    package: Option<&str>,
    limit: i64,
) -> Result<Vec<ReferenceRow>> {
    let mut sql = String::from(
        "SELECT name, kind, file_path, line, package, enclosing_symbol \
         FROM symbol_refs WHERE name = ?1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(name.to_string())];

    if let Some(k) = kind {
        sql.push_str(" AND kind = ?");
        sql.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(k.to_string()));
    }
    if let Some(p) = package {
        sql.push_str(" AND package = ?");
        sql.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(p.to_string()));
    }
    sql.push_str(" ORDER BY file_path, line LIMIT ?");
    sql.push_str(&(params.len() + 1).to_string());
    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(ReferenceRow {
            name: row.get(0)?,
            kind: row.get(1)?,
            file_path: row.get(2)?,
            line: row.get(3)?,
            package: row.get(4)?,
            enclosing_symbol: row.get(5)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
```

- [ ] **Step 2: Add the MCP tool**

In `src/mcp/tools.rs`, find the last `#[tool(...)]` method inside the `impl ShireService` block (look for `fn explore(` around line 590). Add this method right before the last closing brace of that impl block:

```rust
#[tool(description = "Find all references (call sites, type uses, imports, impl clauses) to a symbol by name. Use instead of Grep for 'who uses X?' — returns file, line, kind, and enclosing symbol. Note: matches by name only, so two symbols with the same name cannot be distinguished.")]
fn symbol_references(
    &self,
    Parameters(args): Parameters<SymbolRefsArgs>,
) -> Result<CallToolResult, ErrorData> {
    tracing::debug!(tool = "symbol_references", name = %args.name);
    self.maybe_reindex();
    let conn = self.conn.lock().unwrap();
    let limit = args.limit.unwrap_or(100);
    let rows = queries::query_symbol_references(
        &conn,
        &args.name,
        args.kind.as_deref(),
        args.package.as_deref(),
        limit,
    )
    .map_err(|e| ErrorData::internal_error(Cow::Owned(e.to_string()), None))?;
    let json = serde_json::to_string(&rows)
        .map_err(|e| ErrorData::internal_error(Cow::Owned(e.to_string()), None))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
```

- [ ] **Step 3: Add the parameters struct**

Near the other `...Args` structs in `src/mcp/tools.rs` (grep for `schemars::JsonSchema` or existing `#[derive(Deserialize, schemars::JsonSchema)]` structs), add:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolRefsArgs {
    /// The symbol name to find references for
    pub name: String,
    /// Optional kind filter: "call", "type", "import", or "impl"
    #[serde(default)]
    pub kind: Option<String>,
    /// Optional package filter
    #[serde(default)]
    pub package: Option<String>,
    /// Max results (default 100)
    #[serde(default)]
    pub limit: Option<i64>,
}
```

- [ ] **Step 4: Build**

Run: `cargo build 2>&1 | tail -20`
Expected: No errors.

- [ ] **Step 5: Add a unit test**

The MCP tool test can live in an integration test against a temp DB. For now, test the DB query directly:

Append to `src/db/queries.rs` test module:

```rust
#[test]
fn test_query_symbol_references_filters() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("r.db");
    let conn = open_or_create(&db_path, false).unwrap();

    // Insert fixtures
    conn.execute(
        "INSERT INTO symbol_refs (name, kind, file_path, line, package, enclosing_symbol) \
         VALUES ('foo', 'call', 'a.rs', 10, 'pkg1', 'bar'), \
                ('foo', 'type', 'a.rs', 20, 'pkg1', 'bar'), \
                ('foo', 'call', 'b.rs', 5, 'pkg2', 'quux'), \
                ('other', 'call', 'a.rs', 30, 'pkg1', NULL)",
        [],
    )
    .unwrap();

    // All refs for foo
    let all = query_symbol_references(&conn, "foo", None, None, 100).unwrap();
    assert_eq!(all.len(), 3);

    // Call-only filter
    let calls = query_symbol_references(&conn, "foo", Some("call"), None, 100).unwrap();
    assert_eq!(calls.len(), 2);

    // Package filter
    let p1 = query_symbol_references(&conn, "foo", None, Some("pkg1"), 100).unwrap();
    assert_eq!(p1.len(), 2);

    // Both filters
    let p1_calls = query_symbol_references(&conn, "foo", Some("call"), Some("pkg1"), 100).unwrap();
    assert_eq!(p1_calls.len(), 1);
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test --lib queries::refs_tests 2>&1 | tail -15`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/mcp/tools.rs src/db/queries.rs
git commit -m "feat(mcp): add symbol_references tool"
```

---

### Task 16: `symbol_callers` MCP tool

**Files:**
- Modify: `src/mcp/tools.rs`
- Modify: `src/db/queries.rs`

- [ ] **Step 1: Add the DB query**

Append to `src/db/queries.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct CallerRow {
    pub caller_name: String,
    pub caller_file: String,
    pub caller_line: i64,
    pub call_sites: i64,
}

pub fn query_symbol_callers(
    conn: &Connection,
    name: &str,
    package: Option<&str>,
    limit: i64,
) -> Result<Vec<CallerRow>> {
    let mut sql = String::from(
        "SELECT enclosing_symbol, file_path, MIN(line), COUNT(*) \
         FROM symbol_refs \
         WHERE name = ?1 AND kind = 'call' AND enclosing_symbol IS NOT NULL",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(name.to_string())];
    if let Some(p) = package {
        sql.push_str(" AND package = ?");
        sql.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(p.to_string()));
    }
    sql.push_str(" GROUP BY enclosing_symbol, file_path ORDER BY 4 DESC, 1 ASC LIMIT ?");
    sql.push_str(&(params.len() + 1).to_string());
    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(CallerRow {
            caller_name: row.get(0)?,
            caller_file: row.get(1)?,
            caller_line: row.get(2)?,
            call_sites: row.get(3)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
```

- [ ] **Step 2: Add the MCP tool**

In `src/mcp/tools.rs`:

```rust
#[tool(description = "Find which symbols (functions, methods) call the named symbol. Returns the caller name, file, line of first call, and count of call sites. Navigates the call graph upward.")]
fn symbol_callers(
    &self,
    Parameters(args): Parameters<SymbolCallersArgs>,
) -> Result<CallToolResult, ErrorData> {
    tracing::debug!(tool = "symbol_callers", name = %args.name);
    self.maybe_reindex();
    let conn = self.conn.lock().unwrap();
    let limit = args.limit.unwrap_or(100);
    let rows = queries::query_symbol_callers(&conn, &args.name, args.package.as_deref(), limit)
        .map_err(|e| ErrorData::internal_error(Cow::Owned(e.to_string()), None))?;
    let json = serde_json::to_string(&rows)
        .map_err(|e| ErrorData::internal_error(Cow::Owned(e.to_string()), None))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
```

- [ ] **Step 3: Add the parameters struct**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolCallersArgs {
    /// The symbol being called
    pub name: String,
    /// Optional: restrict callers to this package
    #[serde(default)]
    pub package: Option<String>,
    /// Max results (default 100)
    #[serde(default)]
    pub limit: Option<i64>,
}
```

- [ ] **Step 4: Add unit test**

Append to the queries test module:

```rust
#[test]
fn test_query_symbol_callers_groups_and_counts() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("c.db");
    let conn = open_or_create(&db_path, false).unwrap();

    conn.execute(
        "INSERT INTO symbol_refs (name, kind, file_path, line, package, enclosing_symbol) \
         VALUES ('foo', 'call', 'a.rs', 10, 'p', 'bar'), \
                ('foo', 'call', 'a.rs', 11, 'p', 'bar'), \
                ('foo', 'call', 'b.rs', 5, 'p', 'quux'), \
                ('foo', 'type', 'a.rs', 20, 'p', 'bar'), \
                ('foo', 'call', 'c.rs', 1, 'p', NULL)",
        [],
    ).unwrap();

    let callers = query_symbol_callers(&conn, "foo", None, 100).unwrap();
    // Two distinct callers (bar, quux); NULL enclosing_symbol excluded
    assert_eq!(callers.len(), 2);
    let bar = callers.iter().find(|c| c.caller_name == "bar").unwrap();
    assert_eq!(bar.call_sites, 2);
    assert_eq!(bar.caller_file, "a.rs");
    assert_eq!(bar.caller_line, 10);
}
```

- [ ] **Step 5: Build and run**

Run: `cargo test --lib queries::refs_tests 2>&1 | tail -15`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools.rs src/db/queries.rs
git commit -m "feat(mcp): add symbol_callers tool"
```

---

### Task 17: `symbol_callees` MCP tool

**Files:**
- Modify: `src/mcp/tools.rs`
- Modify: `src/db/queries.rs`

- [ ] **Step 1: Add the DB query**

Append to `src/db/queries.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct CalleeRow {
    pub callee_name: String,
    pub call_sites: i64,
}

pub fn query_symbol_callees(
    conn: &Connection,
    enclosing: &str,
    package: Option<&str>,
    limit: i64,
) -> Result<Vec<CalleeRow>> {
    let mut sql = String::from(
        "SELECT name, COUNT(*) FROM symbol_refs \
         WHERE enclosing_symbol = ?1 AND kind = 'call'",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(enclosing.to_string())];
    if let Some(p) = package {
        sql.push_str(" AND package = ?");
        sql.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(p.to_string()));
    }
    sql.push_str(" GROUP BY name ORDER BY 2 DESC, 1 ASC LIMIT ?");
    sql.push_str(&(params.len() + 1).to_string());
    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(CalleeRow {
            callee_name: row.get(0)?,
            call_sites: row.get(1)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
```

- [ ] **Step 2: Add the MCP tool**

```rust
#[tool(description = "Find which symbols are called from inside the named function/method. Navigates the call graph downward.")]
fn symbol_callees(
    &self,
    Parameters(args): Parameters<SymbolCalleesArgs>,
) -> Result<CallToolResult, ErrorData> {
    tracing::debug!(tool = "symbol_callees", name = %args.name);
    self.maybe_reindex();
    let conn = self.conn.lock().unwrap();
    let limit = args.limit.unwrap_or(100);
    let rows = queries::query_symbol_callees(&conn, &args.name, args.package.as_deref(), limit)
        .map_err(|e| ErrorData::internal_error(Cow::Owned(e.to_string()), None))?;
    let json = serde_json::to_string(&rows)
        .map_err(|e| ErrorData::internal_error(Cow::Owned(e.to_string()), None))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
```

- [ ] **Step 3: Add the parameters struct**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolCalleesArgs {
    /// The caller symbol (function/method name)
    pub name: String,
    /// Optional: restrict to this package
    #[serde(default)]
    pub package: Option<String>,
    /// Max results (default 100)
    #[serde(default)]
    pub limit: Option<i64>,
}
```

- [ ] **Step 4: Add unit test**

```rust
#[test]
fn test_query_symbol_callees_groups() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("cee.db");
    let conn = open_or_create(&db_path, false).unwrap();

    conn.execute(
        "INSERT INTO symbol_refs (name, kind, file_path, line, package, enclosing_symbol) \
         VALUES ('foo', 'call', 'a.rs', 1, 'p', 'handler'), \
                ('bar', 'call', 'a.rs', 2, 'p', 'handler'), \
                ('foo', 'call', 'a.rs', 3, 'p', 'handler'), \
                ('baz', 'call', 'a.rs', 4, 'p', 'other')",
        [],
    ).unwrap();

    let callees = query_symbol_callees(&conn, "handler", None, 100).unwrap();
    assert_eq!(callees.len(), 2);
    let foo = callees.iter().find(|c| c.callee_name == "foo").unwrap();
    assert_eq!(foo.call_sites, 2);
}
```

- [ ] **Step 5: Build and test**

Run: `cargo test --lib queries::refs_tests 2>&1 | tail -15`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools.rs src/db/queries.rs
git commit -m "feat(mcp): add symbol_callees tool"
```

---

### Task 18: `reference_audit` prompt template

**Files:**
- Modify: `src/mcp/prompts.rs`

- [ ] **Step 1: Inspect existing prompts**

Run: `grep -n "fn " src/mcp/prompts.rs | head -20`
Expected: list of existing prompt functions to match the style.

- [ ] **Step 2: Add the `reference_audit` prompt**

The exact shape depends on how existing prompts are registered. Find the pattern used by an existing prompt (e.g., whatever prompt is currently exposed), then mirror it with this content:

```
Title: Reference Audit

Description: Analyze all references to a symbol to assess the safety of renaming, refactoring, or removing it.

Template:
You are auditing references to the symbol `{name}` to support a refactoring decision.

1. Call `symbol_references` with name=`{name}` to get all references.
2. Classify each reference:
   - Call sites (kind=call): these will break if the signature changes.
   - Type references (kind=type): these will break if the type shape changes.
   - Imports (kind=import): these will break if the symbol is moved/removed.
   - Impl clauses (kind=impl): these will break if inheritance contracts change.
3. For each call site, call `symbol_callers` on the enclosing symbol to understand the chain upward.
4. For each call site, check if the enclosing symbol and the referenced symbol are in different packages.
5. Summarize: direct call sites, external callers, implementers, and assess rename/change safety.

Known limitation: name matches only by name, so if two different symbols share the same name across packages, refs to both will appear. Use the `package` field to distinguish.
```

The exact function signature and registration depends on the pattern in prompts.rs — add a matching function.

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -10`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/mcp/prompts.rs
git commit -m "feat(mcp): add reference_audit prompt"
```

---

## Phase 5: Integration and documentation

### Task 19: End-to-end integration test

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: Look at existing integration test patterns**

Run: `grep -n "fn test_" tests/integration.rs | head -10`

Find an existing pattern that: creates a fixture dir, runs `shire build`, opens the DB, and asserts on query results. Mirror it.

- [ ] **Step 2: Add an integration test for references**

Append to `tests/integration.rs`:

```rust
#[test]
fn test_cross_reference_index_end_to_end() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let root = dir.path();

    // Create a minimal Go package with call references
    std::fs::create_dir_all(root.join("svc")).unwrap();
    std::fs::write(
        root.join("svc/go.mod"),
        "module svc\n\ngo 1.22\n",
    ).unwrap();
    std::fs::write(
        root.join("svc/main.go"),
        r#"package svc

func ParseConfig(raw string) Config {
    return Config{}
}

func Handle(req string) {
    cfg := ParseConfig(req)
    Validate(cfg)
}

func Validate(c Config) {}

type Config struct{}
"#,
    ).unwrap();

    // Run shire build
    run_shire(&["build", "--root", root.to_str().unwrap()])
        .expect("build should succeed");

    // Open DB and assert on symbol_refs
    let db_path = root.join(".shire/shire.db");
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();

    let calls_to_parse: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'ParseConfig' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(calls_to_parse >= 1, "expected a call-ref to ParseConfig");

    let enclosing: String = conn
        .query_row(
            "SELECT enclosing_symbol FROM symbol_refs \
             WHERE name = 'ParseConfig' AND kind = 'call' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(enclosing, "Handle");

    let calls_to_validate: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'Validate' AND kind = 'call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(calls_to_validate >= 1);
}
```

Use the existing `run_shire` helper (or whatever test runner is already defined). If it doesn't exist, find the pattern another integration test uses (likely `Command::new(...)` on the built binary).

- [ ] **Step 3: Run the integration test**

Run: `cargo test --test integration test_cross_reference_index_end_to_end 2>&1 | tail -30`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/integration.rs
git commit -m "test: integration test for cross-reference index"
```

---

### Task 20: Incremental rebuild test

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: Add incremental test**

Append to `tests/integration.rs`:

```rust
#[test]
fn test_references_incremental_rebuild() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let root = dir.path();

    std::fs::create_dir_all(root.join("svc")).unwrap();
    std::fs::write(root.join("svc/go.mod"), "module svc\n\ngo 1.22\n").unwrap();
    std::fs::write(
        root.join("svc/a.go"),
        r#"package svc

func A() { B() }
func B() {}
"#,
    ).unwrap();

    run_shire(&["build", "--root", root.to_str().unwrap()]).expect("first build");

    let db_path = root.join(".shire/shire.db");
    let count_b_before: i64 = {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'B' AND kind = 'call'",
            [],
            |r| r.get(0),
        ).unwrap()
    };
    assert_eq!(count_b_before, 1);

    // Modify file: remove the call to B()
    std::fs::write(
        root.join("svc/a.go"),
        r#"package svc

func A() {}
func B() {}
"#,
    ).unwrap();

    run_shire(&["build", "--root", root.to_str().unwrap()]).expect("incremental build");

    let count_b_after: i64 = {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM symbol_refs WHERE name = 'B' AND kind = 'call'",
            [],
            |r| r.get(0),
        ).unwrap()
    };
    assert_eq!(count_b_after, 0, "call-ref to B should be removed after file modification");
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test integration test_references_incremental_rebuild 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test: incremental rebuild keeps references in sync"
```

---

### Task 21: Documentation updates

**Files:**
- Modify: `docs/src/mcp-tools.md`
- Modify: `docs/src/architecture.md`
- Modify: `docs/src/ecosystems.md`
- Modify: `CLAUDE.md`
- Modify: `README.md`

- [ ] **Step 1: Add three tool entries to `docs/src/mcp-tools.md`**

Open the file and match the existing tool-entry format. Append:

```markdown
### `symbol_references`

Find all references (call sites, type uses, imports, impl clauses) to a symbol by name.

**Parameters:**
- `name` (required) — symbol name
- `kind` (optional) — `call` | `type` | `import` | `impl`
- `package` (optional) — filter to refs in this package
- `limit` (optional, default 100)

**Returns:** list of `{ name, kind, file_path, line, package, enclosing_symbol }`.

Name-based matching: two symbols with the same name cannot be distinguished.

### `symbol_callers`

List functions/methods that call the named symbol. Navigates the call graph upward.

**Parameters:**
- `name` (required)
- `package` (optional)
- `limit` (optional, default 100)

**Returns:** list of `{ caller_name, caller_file, caller_line, call_sites }`.

### `symbol_callees`

List symbols called from inside the named function/method. Navigates the call graph downward.

**Parameters:**
- `name` (required)
- `package` (optional)
- `limit` (optional, default 100)

**Returns:** list of `{ callee_name, call_sites }`.
```

- [ ] **Step 2: Update `docs/src/architecture.md`**

Find the `symbols/` section description and append ", plus cross-references (call sites, type uses, imports, impl clauses) for tier 1 languages: Go, Python, Java, TS, JS, Perl, Ruby, Scala."

Add a brief new section for the references table:

```markdown
### symbol_refs table

Cross-reference index. One row per call/type/import/impl reference extracted from source files.
Columns: `name`, `kind`, `file_path`, `line`, `package`, `enclosing_symbol`.
FTS5 index on `name`. Incremental — file-hash-keyed DELETE + bulk INSERT per changed file.
```

- [ ] **Step 3: Update `docs/src/ecosystems.md`**

Add a new "Reference extraction" table matching the style of the existing "Symbol extraction" table:

```markdown
## Reference extraction

| Language | Call | Type | Import | Impl |
|---|---|---|---|---|
| Go | yes | yes | yes | — (implicit interfaces) |
| Python | yes | yes | yes | yes |
| Java | yes | yes | yes | yes |
| TypeScript | yes | yes | yes | yes |
| JavaScript | yes | — | yes | yes |
| Perl | yes | — | yes | — |
| Ruby | yes | yes | yes | yes |
| Scala | yes | yes | yes | yes |

All other languages: symbol definitions only; references not extracted.
```

- [ ] **Step 4: Update `CLAUDE.md`**

Find the `symbols/` description in the "Key modules" section and append "Also extracts cross-references (call/type/import/impl) for Go, Python, Java, TS, JS, Perl, Ruby, Scala via `@reference.*` captures in the same `.scm` files."

Find the `db/` description and append "The `symbol_refs` table carries cross-references, with an FTS5 index on `name` and incremental rebuild at file granularity."

Find the `mcp/` description and update the tool count: "11 tools" → "14 tools".

Add a new "Adding a new reference extractor" subsection under "Adding a new symbol extractor":

```markdown
### Adding reference extraction to a language

Only applicable to languages that already have tree-sitter-based symbol extraction.

1. Add `@reference.call`, `@reference.type`, `@reference.import`, `@reference.impl` captures to the language's `.scm` file alongside existing `@definition.X` captures
2. In the language's hooks file (`src/symbols/hooks/<lang>.rs`), set `enclosing_ancestors: &[...]` with the grammar's function/method/class node kinds
3. Set `reference_stoplist: &[...]` with language built-ins that should be skipped
4. Add unit tests in `src/symbols/tests.rs` asserting each ref kind is extracted
5. Add a row to the Reference extraction table in `docs/src/ecosystems.md`
```

- [ ] **Step 5: Update `README.md`**

In the MCP tools list or feature list, add a brief mention of the three new tools.

- [ ] **Step 6: Build the docs (optional)**

If mdBook is installed: `(cd docs && mdbook build)` — otherwise skip.

- [ ] **Step 7: Commit**

```bash
git add docs/src/mcp-tools.md docs/src/architecture.md docs/src/ecosystems.md CLAUDE.md README.md
git commit -m "docs: cross-reference index tools and architecture"
```

---

## Final verification

- [ ] **Run the full test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: All tests pass — unit + integration.

- [ ] **Run on the shire codebase itself**

```bash
cargo run --release -- build --root .
cargo run --release -- serve  # in another terminal, or skip
```

Then query the DB directly to sanity-check:

```bash
sqlite3 .shire/shire.db "SELECT name, kind, enclosing_symbol FROM symbol_refs WHERE name = 'extract_file' LIMIT 5;"
```

Expected: several call-refs to `extract_file` across the codebase with enclosing symbols populated.

- [ ] **Verify the binary output format**

Start the MCP server in a test harness and verify `symbol_references`, `symbol_callers`, `symbol_callees` return well-formed JSON.

---

## Notes for the implementer

- Each task ends with a commit; don't batch commits across tasks.
- Tree-sitter grammars vary in node-name conventions. If a test fails because a query returns no matches, inspect the parse tree by temporarily adding `eprintln!("{}", tree.root_node().to_sexp());` inside `query_extract::extract()` and re-running the failing test.
- The `reference_stoplist` entries can be tuned as you go — start conservative and add entries if you see noise in `symbol_references` results.
- When in doubt about a grammar, run `tree-sitter parse <file>` from CLI (if installed) against a sample source file.
- Don't forget to run `cargo clippy -- -W clippy::all 2>&1 | tail -20` before the final verification — clippy often catches clumsy error handling.
