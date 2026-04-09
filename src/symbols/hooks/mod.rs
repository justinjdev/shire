pub mod bash;
pub mod c;
pub mod clojure;
pub mod cpp;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod erlang;
pub mod gleam;
pub mod go;
pub mod haskell;
pub mod hcl;
pub mod java;
pub mod javascript;
pub mod julia;
pub mod kotlin;
pub mod lua;
pub mod nim;
pub mod nix;
pub mod ocaml;
pub mod odin;
pub mod perl;
pub mod php;
pub mod proto;
pub mod python;
pub mod r;
pub mod ruby;
pub mod rust_lang;
pub mod scala;
pub mod sql;
pub mod swift;
// Named toml_lang to avoid collision with the `toml` crate
pub mod toml_lang;
pub mod typescript;
pub mod yaml;
pub mod zig;

use super::{Parameter, SymbolInfo, SymbolKind, Visibility};
use tree_sitter::Node;

/// Function pointer type for building a symbol's signature from its definition node.
pub type SignatureFn = fn(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String;

/// Function pointer type for post-processing a matched symbol; returning None drops it.
pub type PostProcessFn = fn(sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo>;

/// Hooks for reference extraction. Only populated for languages with
/// cross-reference support (currently: Go, Python, Java, TypeScript,
/// JavaScript, Perl, Ruby, Scala).
pub struct ReferenceHooks {
    /// Node kinds that qualify as an enclosing symbol for references.
    pub enclosing_ancestors: &'static [&'static str],
    /// Identifiers to skip when emitting references (language built-ins, etc.).
    pub reference_stoplist: &'static [&'static str],
}

/// Hooks for language-specific symbol enrichment.
/// All fields are optional — None means use the default behavior.
#[derive(Default)]
pub struct LanguageHooks {
    /// Filter: return true if the symbol should be included (is visible/exported).
    /// Receives the definition node and source. Default: include all.
    pub is_visible: Option<fn(node: &Node, source: &str) -> bool>,

    /// Resolve parent symbol name (e.g., class name for methods, impl target for Rust).
    /// Receives the definition node and source. Default: None (no parent).
    pub resolve_parent: Option<fn(node: &Node, source: &str) -> Option<String>>,

    /// Build signature string. Receives definition node, source, name, and kind.
    /// Default: "kind name" (e.g., "class Foo").
    pub build_signature: Option<SignatureFn>,

    /// Extract function/method parameters. Receives definition node and source.
    /// Default: empty vec.
    pub extract_parameters: Option<fn(node: &Node, source: &str) -> Vec<Parameter>>,

    /// Extract return type. Receives definition node and source.
    /// Default: None.
    pub extract_return_type: Option<fn(node: &Node, source: &str) -> Option<String>>,

    /// Post-process a matched symbol before adding to results.
    /// Return None to skip the symbol.
    pub post_process: Option<PostProcessFn>,

    /// Reference extraction hooks. `None` means this language has no
    /// cross-reference support — the extractor skips ref processing entirely.
    pub reference_hooks: Option<ReferenceHooks>,
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

/// Walk up from `node` through ancestors looking for the first node whose kind
/// is listed in `ancestors`. Returns the text of that ancestor's `name` field
/// (or its first `identifier`/`type_identifier` child) as the enclosing symbol
/// name. Returns None if no qualifying named ancestor is found.
pub fn resolve_enclosing_symbol(node: &Node, source: &str, ancestors: &[&str]) -> Option<String> {
    fn is_anonymous_callable_kind(kind: &str) -> bool {
        matches!(
            kind,
            "arrow_function" | "function_expression" | "lambda" | "lambda_expression"
        )
    }

    let mut current = node.parent();
    while let Some(n) = current {
        if ancestors.contains(&n.kind()) {
            // Try the `name` field first (most grammars)
            if let Some(name) = field_text(&n, "name", source) {
                return Some(name.to_string());
            }
            // Anonymous callable scopes should still be attributable, but they
            // don't have a stable identifier node to read as a name.
            if is_anonymous_callable_kind(n.kind()) {
                return Some("<anonymous>".to_string());
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
