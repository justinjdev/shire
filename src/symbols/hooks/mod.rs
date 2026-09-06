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

fn is_anonymous_callable_kind(kind: &str) -> bool {
    matches!(
        kind,
        "arrow_function" | "function_expression" | "lambda" | "lambda_expression"
    )
}

/// Name an anonymous callable (arrow function / function expression / lambda)
/// from the binding it is immediately assigned to, e.g. `const f = () => ...`,
/// `{ f: () => ... }`, `this.f = () => ...`, or a class field `f = () => ...`.
/// Falls back to `<anonymous>` when no such binding exists (e.g. an inline
/// callback passed directly as a call argument).
fn name_anonymous_callable(n: &Node, source: &str) -> String {
    if let Some(parent) = n.parent() {
        let bound_name = match parent.kind() {
            "variable_declarator" => field_text(&parent, "name", source),
            "pair" => field_text(&parent, "key", source),
            "assignment_expression" => field_text(&parent, "left", source),
            // JS class fields use `field_definition` (field `property`); the
            // TS grammar instead uses `public_field_definition` (field `name`).
            "field_definition" => field_text(&parent, "property", source),
            "public_field_definition" => field_text(&parent, "name", source),
            _ => None,
        };
        if let Some(name) = bound_name {
            return name.to_string();
        }
    }
    "<anonymous>".to_string()
}

/// Name a single enclosing-ancestor node: its `name` field (most grammars),
/// a derived binding name for anonymous callables, or the first
/// identifier-like child as a fallback. Returns None if the node cannot be
/// named at all (in which case the caller keeps climbing past it).
fn enclosing_ancestor_name(n: &Node, source: &str) -> Option<String> {
    if let Some(name) = field_text(n, "name", source) {
        return Some(name.to_string());
    }
    if is_anonymous_callable_kind(n.kind()) {
        return Some(name_anonymous_callable(n, source));
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
    None
}

/// Walk up from `node` through ancestors, collecting the name of every node
/// whose kind is listed in `ancestors`, and join them innermost-last with `.`
/// (e.g. `Class.method`) so that callers with the same name in different
/// scopes don't collapse into one `enclosing_symbol`. Anonymous callables
/// (arrow functions, function expressions, lambdas) are named from the
/// binding they are assigned to rather than reported as a bare
/// `"<anonymous>"`. Returns None if no qualifying ancestor is found at all.
pub fn resolve_enclosing_symbol(node: &Node, source: &str, ancestors: &[&str]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = node.parent();
    while let Some(n) = current {
        if ancestors.contains(&n.kind())
            && let Some(name) = enclosing_ancestor_name(&n, source)
        {
            parts.push(name);
        }
        current = n.parent();
    }
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    Some(parts.join("."))
}
