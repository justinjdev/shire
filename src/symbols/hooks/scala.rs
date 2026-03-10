use super::{find_ancestor, find_child_by_kind, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Scala type-defining node kinds.
const TYPE_NODES: &[&str] = &[
    "class_definition",
    "object_definition",
    "trait_definition",
    "enum_definition",
];

/// Check whether a Scala symbol is visible (not private).
/// Scala defaults to public visibility.
/// Also checks ancestor class/object/trait visibility — methods inside a private
/// type are not visible.
fn is_visible(node: &Node, source: &str) -> bool {
    if has_private_modifier(node, source) {
        return false;
    }

    // Check all ancestor types for visibility
    let mut current = node.parent();
    while let Some(n) = current {
        if TYPE_NODES.contains(&n.kind()) && has_private_modifier(&n, source) {
            return false;
        }
        current = n.parent();
    }

    true
}

/// Check if a node has a `private` access modifier.
/// In this grammar, access_modifier lives inside a `modifiers` wrapper node.
fn has_private_modifier(node: &Node, source: &str) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "access_modifier" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                if text.starts_with("private") {
                    return true;
                }
            }
        }
        // The grammar wraps access_modifier inside a `modifiers` node
        if child.kind() == "modifiers" {
            for j in 0..child.child_count() {
                let grandchild = child.child(j).unwrap();
                if grandchild.kind() == "access_modifier" {
                    if let Ok(text) = grandchild.utf8_text(source.as_bytes()) {
                        if text.starts_with("private") {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// For methods inside class/object/trait/enum bodies, resolve the parent type name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    let kind = node.kind();
    if kind != "function_definition" && kind != "function_declaration" {
        return None;
    }

    // Methods live inside template_body or enum_body; walk up to find the enclosing type.
    for &type_node in TYPE_NODES {
        if let Some(parent) = find_ancestor(node, type_node) {
            return parent
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());
        }
    }

    None
}

/// Build a signature string for a Scala symbol.
///
/// For functions/methods: source span from node start up to (but not including) the body.
/// For types: "class Name", "object Name", "trait Name", "enum Name".
/// For type aliases: "type Name".
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let start = node.start_byte();
            let end = find_child_by_kind(node, "block")
                .or_else(|| find_child_by_kind(node, "indented_block"))
                .or_else(|| find_child_by_kind(node, "="))
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        _ => {
            let keyword = detect_type_keyword(node);
            format!("{} {}", keyword, name)
        }
    }
}

/// Extract parameters from a `parameters` node.
///
/// Each `parameter` child has a `name` (identifier) and optionally a colon followed
/// by a type annotation.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter" {
            let name = find_param_name(source, &child).unwrap_or_default();
            let type_ann = extract_parameter_type(source, &child);

            if !name.is_empty() {
                params.push(Parameter {
                    name,
                    type_annotation: type_ann,
                });
            }
        }
    }

    params
}

/// Find the identifier name inside a parameter node.
fn find_param_name(source: &str, param_node: &Node) -> Option<String> {
    param_node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
        .or_else(|| {
            // Fallback: look for first identifier child
            for i in 0..param_node.child_count() {
                let child = param_node.child(i).unwrap();
                if child.kind() == "identifier" {
                    return child
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                }
            }
            None
        })
}

/// Extract the type annotation from a `parameter` node.
/// A parameter is structured as: name ":" type
fn extract_parameter_type(source: &str, param_node: &Node) -> Option<String> {
    let mut found_colon = false;
    for i in 0..param_node.child_count() {
        let child = param_node.child(i).unwrap();
        if child.kind() == ":" {
            found_colon = true;
            continue;
        }
        if found_colon {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Extract the return type from a function definition/declaration.
/// The return type follows a ":" after the `parameters` node.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Detect the type keyword for a node.
fn detect_type_keyword(node: &Node) -> &'static str {
    match node.kind() {
        "object_definition" => "object",
        "trait_definition" => "trait",
        "enum_definition" => "enum",
        "type_definition" => "type",
        "class_definition" => "class",
        _ => "class",
    }
}

/// Post-process: for `object_definition` nodes, keep as Class kind (signature already
/// shows "object Name"). For `trait_definition`, ensure kind is Interface.
/// For `enum_definition`, ensure kind is Enum.
fn post_process(mut sym: SymbolInfo, node: &Node, _source: &str) -> Option<SymbolInfo> {
    match node.kind() {
        "trait_definition" => {
            sym.kind = SymbolKind::Interface;
        }
        "enum_definition" => {
            sym.kind = SymbolKind::Enum;
        }
        "type_definition" => {
            sym.kind = SymbolKind::Type;
        }
        "object_definition" => {
            // Keep as Class — signature already reads "object Name"
        }
        _ => {}
    }
    Some(sym)
}

/// Return Scala language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
    }
}
