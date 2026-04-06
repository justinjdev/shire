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

/// Trim surrounding quotes from import reference names. Several grammars
/// expose import paths as string-literal nodes with quote characters
/// included (e.g. Go `"fmt"`, Ruby `'json'`).
fn normalize_import_name(name: &str, kind: ReferenceKind) -> String {
    if matches!(kind, ReferenceKind::Import) {
        name.trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
            .to_string()
    } else {
        name.to_string()
    }
}

/// Build a SymbolInfo from a classified definition match.
/// Returns None if the symbol should be skipped (duplicate range, visibility filter, post-process).
fn emit_definition(
    name: &str,
    kind: SymbolKind,
    node: &tree_sitter::Node,
    name_node: &tree_sitter::Node,
    source: &str,
    file_path: &Arc<str>,
    hooks: &LanguageHooks,
    seen_def_ranges: &mut std::collections::HashSet<(usize, usize)>,
    def_name_ranges: &mut std::collections::HashSet<(usize, usize)>,
) -> Option<SymbolInfo> {
    let range_key = (node.start_byte(), node.end_byte());
    if !seen_def_ranges.insert(range_key) {
        return None;
    }
    def_name_ranges.insert((name_node.start_byte(), name_node.end_byte()));

    if let Some(is_visible) = hooks.is_visible {
        if !is_visible(node, source) {
            return None;
        }
    }
    let line = node.start_position().row + 1;
    let parent = hooks.resolve_parent.and_then(|f| f(node, source));
    let signature = hooks
        .build_signature
        .map(|f| f(node, source, name, kind))
        .unwrap_or_else(|| default_signature(name, kind));
    let parameters = if matches!(kind, SymbolKind::Function | SymbolKind::Method) {
        Some(
            hooks
                .extract_parameters
                .map(|f| f(node, source))
                .unwrap_or_default(),
        )
    } else {
        None
    };
    let return_type = if matches!(kind, SymbolKind::Function | SymbolKind::Method) {
        hooks.extract_return_type.and_then(|f| f(node, source))
    } else {
        None
    };
    let sym = SymbolInfo {
        name: name.to_string(),
        kind,
        signature: Some(signature),
        file_path: file_path.clone(),
        line,
        visibility: Visibility::Public,
        parent_symbol: parent,
        return_type,
        parameters,
    };
    if let Some(post) = hooks.post_process {
        post(sym, node, source)
    } else {
        Some(sym)
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
            let mut matches = cursor.matches(query, tree.root_node(), source_bytes);
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
                if def_kind.is_none()
                    && let Some(k) = capture_name_to_kind(cname) {
                        def_kind = Some(k);
                        def_node = Some(capture.node);
                        continue;
                    }
                if ref_kind.is_none()
                    && let Some(k) = capture_name_to_ref_kind(cname) {
                        ref_kind = Some(k);
                        ref_node = Some(capture.node);
                    }
            }

            // Definition path
            if let (Some(kind), Some(node)) = (def_kind, def_node) {
                if let Some(sym) = emit_definition(
                    &name,
                    kind,
                    &node,
                    &name_capture.unwrap().node,
                    source,
                    &file_path,
                    hooks,
                    &mut seen_def_ranges,
                    &mut def_name_ranges,
                ) {
                    symbols.push(sym);
                }
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
                let trimmed_name = normalize_import_name(&name, kind);
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

        let references = filter_references(pending_references, &def_name_ranges);
        (symbols, references)
    })
}

/// Post-pass filter for buffered references. Suppresses:
/// (a) self-references at a definition's own name node,
/// (b) Type refs that duplicate an Impl ref at the same byte range,
/// (c) Type refs that duplicate a Call ref at the same byte range.
fn filter_references(
    pending: Vec<(ReferenceInfo, (usize, usize))>,
    def_name_ranges: &std::collections::HashSet<(usize, usize)>,
) -> Vec<ReferenceInfo> {
    let impl_ranges: std::collections::HashSet<(usize, usize)> = pending
        .iter()
        .filter_map(|(r, range)| {
            if r.kind == ReferenceKind::Impl {
                Some(*range)
            } else {
                None
            }
        })
        .collect();
    let call_ranges: std::collections::HashSet<(usize, usize)> = pending
        .iter()
        .filter_map(|(r, range)| {
            if r.kind == ReferenceKind::Call {
                Some(*range)
            } else {
                None
            }
        })
        .collect();

    pending
        .into_iter()
        .filter_map(|(reference, node_range)| {
            if def_name_ranges.contains(&node_range) {
                return None;
            }
            if reference.kind == ReferenceKind::Type
                && (impl_ranges.contains(&node_range) || call_ranges.contains(&node_range))
            {
                return None;
            }
            Some(reference)
        })
        .collect()
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
