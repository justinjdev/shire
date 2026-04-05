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

    QUERY_CURSOR.with_borrow_mut(|cursor| {
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

            // Determine whether this match is a definition or a reference
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

            // Definition path
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

            // Reference path
            if let (Some(kind), Some(node)) = (ref_kind, ref_node) {
                if hooks.reference_stoplist.contains(&name.as_str()) {
                    continue;
                }
                // Trim surrounding quotes for import names (common for Go/Ruby strings)
                let trimmed_name = if matches!(kind, ReferenceKind::Import) {
                    name.trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
                        .to_string()
                } else {
                    name
                };
                let line = node.start_position().row + 1;
                let enclosing =
                    resolve_enclosing_symbol(&node, source, hooks.enclosing_ancestors);
                references.push(ReferenceInfo {
                    name: trimmed_name,
                    kind,
                    file_path: file_path.clone(),
                    line,
                    enclosing_symbol: enclosing,
                });
            }
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
