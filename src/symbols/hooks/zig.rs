use super::{LanguageHooks, Parameter, SymbolInfo, SymbolKind, find_child_by_kind, node_text};
use tree_sitter::Node;

/// Zig visibility: only symbols marked `pub` are exported.
/// The `pub` keyword appears as an anonymous child node with text "pub".
fn is_visible(node: &Node, source: &str) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.is_named() {
            // Named children come after any leading keywords; stop searching.
            break;
        }
        if node_text(&child, source) == Some("pub") {
            return true;
        }
    }
    false
}

/// Zig doesn't have methods attached to types in the AST (they're just functions
/// that take a self parameter), so we return None.
fn resolve_parent(_node: &Node, _source: &str) -> Option<String> {
    None
}

/// Build a signature string for a Zig symbol.
///
/// For functions: source span from node start to the opening brace of the body.
/// For variable declarations: "pub const Name = <type_keyword>" or "const Name".
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let start = node.start_byte();
            // Find the block child (function body) and stop just before it.
            let end = find_child_by_kind(node, "block")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        SymbolKind::Struct => {
            if is_visible(node, source) {
                format!("pub const {} = struct", name)
            } else {
                format!("const {} = struct", name)
            }
        }
        SymbolKind::Enum => {
            if is_visible(node, source) {
                format!("pub const {} = enum", name)
            } else {
                format!("const {} = enum", name)
            }
        }
        _ => {
            if is_visible(node, source) {
                format!("pub const {}", name)
            } else {
                format!("const {}", name)
            }
        }
    }
}

/// Extract parameters from a Zig function declaration.
/// Zig function parameters live inside a `parameters` field or child node.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    if node.kind() != "function_declaration" {
        return Vec::new();
    }

    // Try field name first, then search by kind.
    let params_node = node
        .child_by_field_name("parameters")
        .or_else(|| find_child_by_kind(node, "parameters"))
        .or_else(|| find_child_by_kind(node, "param_list"));

    let params_node = match params_node {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        // Look for parameter nodes (param_declaration or param)
        let kind = child.kind();
        if kind == "param_declaration" || kind == "param" || kind == "parameter" {
            let param_name = child
                .child_by_field_name("name")
                .or_else(|| find_child_by_kind(&child, "identifier"))
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("")
                .to_string();

            let type_ann = child
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());

            if !param_name.is_empty() {
                params.push(Parameter {
                    name: param_name,
                    type_annotation: type_ann,
                });
            }
        }
    }

    params
}

/// Extract return type from a Zig function declaration.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "function_declaration" {
        return None;
    }

    // Try the return_type field first.
    node.child_by_field_name("return_type")
        .or_else(|| find_child_by_kind(node, "return_type"))
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Post-process: reclassify variable_declaration nodes based on their expression child.
/// If the value is a struct_declaration -> Struct, enum_declaration -> Enum,
/// union_declaration -> Struct, otherwise keep as Constant.
fn post_process(mut sym: SymbolInfo, node: &Node, _source: &str) -> Option<SymbolInfo> {
    if sym.kind == SymbolKind::Constant && node.kind() == "variable_declaration" {
        // Check the expression/value child for struct, enum, or union declarations.
        for i in 0..node.child_count() {
            let child = node.child(i).unwrap();
            match child.kind() {
                "struct_declaration" | "container_declaration" => {
                    // container_declaration is used in some tree-sitter-zig versions
                    sym.kind = SymbolKind::Struct;
                    break;
                }
                "enum_declaration" => {
                    sym.kind = SymbolKind::Enum;
                    break;
                }
                "union_declaration" => {
                    sym.kind = SymbolKind::Struct;
                    break;
                }
                _ => {}
            }
        }
    }

    // For test_declaration nodes, keep them but they won't have a name from the query.
    // Skip test declarations since they don't have a captured @name.
    if node.kind() == "test_declaration" {
        return None;
    }

    Some(sym)
}

/// Return the language hooks for Zig.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        reference_hooks: None,
    }
}
