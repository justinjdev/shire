pub mod bash;
pub mod c;
pub mod cpp;
pub mod csharp;
pub mod go;
pub mod haskell;
pub mod java;
pub mod kotlin;
pub mod php;
pub mod proto;
pub mod python;
pub mod r;
pub mod rust_lang;
pub mod scala;
pub mod swift;
pub mod typescript;
pub mod yaml;
pub mod zig;

use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Hooks for language-specific symbol enrichment.
/// All fields are optional — None means use the default behavior.
pub struct LanguageHooks {
    /// Filter: return true if the symbol should be included (is visible/exported).
    /// Receives the definition node and source. Default: include all.
    pub is_visible: Option<fn(node: &Node, source: &str) -> bool>,

    /// Resolve parent symbol name (e.g., class name for methods, impl target for Rust).
    /// Receives the definition node and source. Default: None (no parent).
    pub resolve_parent: Option<fn(node: &Node, source: &str) -> Option<String>>,

    /// Build signature string. Receives definition node, source, name, and kind.
    /// Default: "kind name" (e.g., "class Foo").
    pub build_signature:
        Option<fn(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String>,

    /// Extract function/method parameters. Receives definition node and source.
    /// Default: empty vec.
    pub extract_parameters: Option<fn(node: &Node, source: &str) -> Vec<Parameter>>,

    /// Extract return type. Receives definition node and source.
    /// Default: None.
    pub extract_return_type: Option<fn(node: &Node, source: &str) -> Option<String>>,

    /// Post-process a matched symbol before adding to results.
    /// Return None to skip the symbol.
    pub post_process:
        Option<fn(sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo>>,
}

impl Default for LanguageHooks {
    fn default() -> Self {
        Self {
            is_visible: None,
            resolve_parent: None,
            build_signature: None,
            extract_parameters: None,
            extract_return_type: None,
            post_process: None,
        }
    }
}

/// Helper: find first child node with the given kind.
pub fn find_child_by_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Helper: get text of a node.
pub fn node_text<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
}

/// Helper: get text of a child by field name.
pub fn field_text<'a>(node: &Node, field: &str, source: &'a str) -> Option<&'a str> {
    node.child_by_field_name(field)
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
}

/// Helper: walk up the tree to find an ancestor with the given kind.
pub fn find_ancestor<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == kind {
            return Some(n);
        }
        current = n.parent();
    }
    None
}
