use std::sync::Arc;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use super::hooks::LanguageHooks;
use super::{SymbolInfo, SymbolKind};

/// Map query capture names to SymbolKind.
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

/// Extract symbols from source using a pre-compiled tree-sitter query and a reusable parser.
///
/// The caller is responsible for compiling the `Query` (once per language) and creating
/// a `Parser` with the correct language set. This avoids per-file compilation overhead.
pub fn extract(
    parser: &mut Parser,
    query: &Query,
    source: &str,
    file_path: Arc<str>,
    hooks: &LanguageHooks,
) -> Vec<SymbolInfo> {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let capture_names = query.capture_names();
    let name_idx = capture_names.iter().position(|&n| n == "name");
    let name_idx = match name_idx {
        Some(i) => i as u32,
        None => return Vec::new(),
    };

    let mut cursor = QueryCursor::new();
    let mut symbols = Vec::new();
    let mut seen_def_ranges = std::collections::HashSet::new();
    let source_bytes = source.as_bytes();

    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);
    while let Some(m) = matches.next() {
        // Find the @name capture text
        let name_capture = m.captures.iter().find(|c| c.index == name_idx);
        let name = match name_capture {
            Some(c) => match c.node.utf8_text(source_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            },
            None => continue,
        };

        // Find the @definition.X capture to determine kind
        let mut kind = None;
        let mut def_node = None;
        for capture in m.captures.iter() {
            let cname = capture_names[capture.index as usize];
            if let Some(k) = capture_name_to_kind(cname) {
                kind = Some(k);
                def_node = Some(capture.node);
                break;
            }
        }

        let (kind, node) = match (kind, def_node) {
            (Some(k), Some(n)) => (k, n),
            _ => continue,
        };

        // Deduplicate: skip if we already matched this definition node byte range
        let range_key = (node.start_byte(), node.end_byte());
        if !seen_def_ranges.insert(range_key) {
            continue;
        }

        // Apply visibility hook
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
            name,
            kind,
            signature: Some(signature),
            file_path: file_path.clone(),
            line,
            visibility: "public".to_string(),
            parent_symbol: parent,
            return_type,
            parameters,
        };

        // Apply post-process hook
        let sym = if let Some(post) = hooks.post_process {
            match post(sym, &node, source) {
                Some(s) => s,
                None => continue,
            }
        } else {
            sym
        };

        symbols.push(sym);
    }

    symbols
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
