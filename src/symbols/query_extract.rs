use std::cell::RefCell;
use std::sync::Arc;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use super::hooks::{resolve_enclosing_symbol, LanguageHooks};
use super::{ReferenceInfo, ReferenceKind, SymbolInfo, SymbolKind, Visibility};

thread_local! {
    /// Per-thread pooled QueryCursor to avoid repeated allocation.
    /// Tree-sitter's QueryCursor holds internal buffers that can be reused
    /// across queries on the same thread.
    static QUERY_CURSOR: RefCell<QueryCursor> = RefCell::new(QueryCursor::new());
}

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

/// Run a compiled tree-sitter query against `source`, returning both the
/// definitions (`SymbolInfo`) and references (`ReferenceInfo`) captured.
///
/// The `Query` is expected to be compiled once per language (cached upstream)
/// and the `Parser` must already have its language set. `@definition.*` captures
/// flow to the symbols vec; `@reference.*` captures flow to references, with
/// stoplist filtering, enclosing-symbol resolution, and self-reference removal
/// (references whose byte range coincides with a definition's name node are
/// dropped — e.g. `type Config struct` does not produce a Config→Config ref).
pub fn extract(
    parser: &mut Parser,
    query: &Query,
    source: &str,
    file_path: Arc<str>,
    hooks: &LanguageHooks,
    skip_references: bool,
) -> (Vec<SymbolInfo>, Vec<ReferenceInfo>) {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            // A parser that returns None for non-empty source means the
            // grammar hit a resource/recursion limit or the input was not
            // valid UTF-8 at a byte tree-sitter cares about. Without this
            // log, the file gets hashed as "processed" with zero symbols
            // and zero refs — indistinguishable from an empty file.
            if !source.is_empty() {
                tracing::warn!(
                    file = %file_path,
                    source_bytes = source.len(),
                    "tree-sitter parse returned None — file yields no symbols or refs"
                );
            }
            return (Vec::new(), Vec::new());
        }
    };

    let capture_names = query.capture_names();
    let name_idx = match capture_names.iter().position(|&n| n == "name") {
        Some(i) => i as u32,
        None => {
            tracing::warn!(
                file = %file_path,
                "tree-sitter query has no @name capture — skipping extraction"
            );
            return (Vec::new(), Vec::new());
        }
    };

    QUERY_CURSOR.with_borrow_mut(|cursor| {
        let mut symbols = Vec::new();
        let mut references = Vec::new();
        let mut seen_def_ranges = std::collections::HashSet::new();
        // Tracks byte ranges of definition NAME nodes so we can suppress
        // reference captures that coincide with a definition's own name node.
        // Tree-sitter may emit a reference match for the same identifier node
        // that was already captured as a definition name, producing spurious
        // self-references (e.g. `type Config struct` → Config type-ref at
        // declaration line).  We buffer pending references and filter them
        // after processing all matches.
        let mut def_name_ranges: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();
        let mut pending_references: Vec<(ReferenceInfo, (usize, usize))> = Vec::new();
        let source_bytes = source.as_bytes();

        // Collect all matches into a Vec so we can do two logical passes
        // without re-running the query (avoids double-parse overhead).
        let all_matches: Vec<_> = {
            let mut buf = Vec::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
            while let Some(m) = matches.next() {
                // Clone captures so we can store them independent of the cursor.
                buf.push(m.captures.to_vec());
            }
            buf
        };

        for captures in &all_matches {
            let name_capture = captures.iter().find(|c| c.index == name_idx);
            let name = match name_capture {
                Some(c) => match c.node.utf8_text(source_bytes) {
                    Ok(s) => s.to_string(),
                    Err(_) => continue,
                },
                None => continue,
            };

            // Determine whether this match is a definition or a reference
            let mut def_kind = None;
            let mut def_node = None;
            let mut ref_kind = None;
            let mut ref_node = None;
            for capture in captures.iter() {
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

            // Definition path
            if let (Some(kind), Some(node)) = (def_kind, def_node) {
                let range_key = (node.start_byte(), node.end_byte());
                if !seen_def_ranges.insert(range_key) {
                    continue;
                }
                // Record the NAME node's byte range so we can suppress any
                // reference capture that lands on the same identifier.
                if let Some(nc) = name_capture {
                    def_name_ranges
                        .insert((nc.node.start_byte(), nc.node.end_byte()));
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
                    visibility: Visibility::Public,
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

            // Reference path — buffer for post-pass filtering
            if skip_references {
                continue;
            }
            if let (Some(kind), Some(node)) = (ref_kind, ref_node) {
                if hooks.reference_stoplist.contains(&name.as_str()) {
                    continue;
                }
                // Trim surrounding quotes for import names. Lives here (not in
                // per-language hooks) because several grammars expose import
                // paths as string-literal nodes with the quote characters
                // included — e.g. Go (`import "fmt"` → `"fmt"`) and Ruby
                // (`require 'json'` → `'json'`). Centralizing keeps language
                // `.scm` files simple.
                let trimmed_name = if matches!(kind, ReferenceKind::Import) {
                    name.trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
                        .to_string()
                } else {
                    name
                };
                let line = node.start_position().row + 1;
                let enclosing =
                    resolve_enclosing_symbol(&node, source, hooks.enclosing_ancestors);
                let node_range = (node.start_byte(), node.end_byte());
                pending_references.push((
                    ReferenceInfo {
                        name: trimmed_name,
                        kind,
                        file_path: file_path.clone(),
                        line,
                        enclosing_symbol: enclosing,
                    },
                    node_range,
                ));
            }
        }

        // Build sets of byte ranges captured as Impl or Call. A node
        // captured as Impl or Call is ALWAYS the right classification for
        // that node — e.g. `BaseService` in `extends BaseService` is an
        // Impl, and a `Constant` in method-call position (Ruby: the bare
        // `(constant) @reference.type` pattern also matches method-name
        // constants captured as `@reference.call` via
        // `(call method: (constant))`) is a Call. But the generic
        // `(type_identifier) @reference.type` / `(constant)
        // @reference.type` patterns also match these nodes, producing
        // duplicate Type rows. Suppress Type refs at node ranges already
        // claimed by Impl or Call.
        let impl_ranges: std::collections::HashSet<(usize, usize)> = pending_references
            .iter()
            .filter_map(|(r, range)| {
                if r.kind == ReferenceKind::Impl {
                    Some(*range)
                } else {
                    None
                }
            })
            .collect();
        let call_ranges: std::collections::HashSet<(usize, usize)> = pending_references
            .iter()
            .filter_map(|(r, range)| {
                if r.kind == ReferenceKind::Call {
                    Some(*range)
                } else {
                    None
                }
            })
            .collect();

        // Emit references, skipping:
        //   (a) self-references at a definition's own name node,
        //   (b) Type refs that duplicate an Impl ref at the same node, and
        //   (c) Type refs that duplicate a Call ref at the same node.
        for (reference, node_range) in pending_references {
            if def_name_ranges.contains(&node_range) {
                continue;
            }
            if reference.kind == ReferenceKind::Type
                && (impl_ranges.contains(&node_range) || call_ranges.contains(&node_range))
            {
                continue;
            }
            references.push(reference);
        }

        (symbols, references)
    })
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
