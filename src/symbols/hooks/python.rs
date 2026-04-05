use super::{field_text, find_ancestor, node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Check if a Python symbol should be included.
///
/// For methods (function_definition inside a class body): skip names starting
/// with `_` except `__init__`. Top-level functions and classes are always visible.
fn is_visible(node: &Node, source: &str) -> bool {
    let name = match node.child_by_field_name("name") {
        Some(n) => match n.utf8_text(source.as_bytes()) {
            Ok(s) => s,
            Err(_) => return true,
        },
        None => return true,
    };

    // Only apply underscore filtering to methods (functions inside a class body)
    if node.kind() == "function_definition" {
        if let Some(parent) = node.parent() {
            // parent is the `block` node; its parent is the `class_definition`
            if parent.kind() == "block" {
                if let Some(grandparent) = parent.parent() {
                    if grandparent.kind() == "class_definition" {
                        // Inside a class: skip _private except __init__
                        if name.starts_with('_') && name != "__init__" {
                            return false;
                        }
                    }
                }
            }
        }
    }

    true
}

/// Resolve the parent symbol (class name) for methods.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    let class_node = find_ancestor(node, "class_definition")?;
    field_text(&class_node, "name", source).map(|s| s.to_string())
}

/// Build signature string for Python symbols.
///
/// Functions/methods: `def name(params) -> ret`
/// Classes: `class Name`
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Class => format!("class {}", name),
        SymbolKind::Function | SymbolKind::Method => {
            let params_text = node
                .child_by_field_name("parameters")
                .and_then(|n| node_text(&n, source))
                .unwrap_or("()");
            let ret = node
                .child_by_field_name("return_type")
                .and_then(|n| node_text(&n, source))
                .map(|r| format!(" -> {}", r))
                .unwrap_or_default();
            format!("def {}{}{}", name, params_text, ret)
        }
        _ => format!("{}", name),
    }
}

/// Extract parameters from a Python function/method definition.
///
/// Handles: `identifier`, `typed_parameter`, `typed_default_parameter`,
/// `default_parameter`. Does NOT filter `self` here — that is handled in
/// post_process so it only applies to methods.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name("parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        match child.kind() {
            "identifier" => {
                let name = child
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() {
                    params.push(Parameter {
                        name,
                        type_annotation: None,
                    });
                }
            }
            "typed_parameter" => {
                let name = child
                    .child(0)
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("")
                    .to_string();
                let type_ann = child
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.to_string());

                if !name.is_empty() {
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            "typed_default_parameter" | "default_parameter" => {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("")
                    .to_string();
                let type_ann = child
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.to_string());

                if !name.is_empty() {
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            _ => {}
        }
    }

    params
}

/// Extract return type from a Python function/method definition.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Post-process: filter `self` from method parameters.
fn post_process(mut sym: SymbolInfo, _node: &Node, _source: &str) -> Option<SymbolInfo> {
    if sym.kind == SymbolKind::Method {
        sym.parameters = sym.parameters.map(|params| {
            params
                .into_iter()
                .filter(|p| p.name != "self")
                .collect()
        });
    }
    Some(sym)
}

/// Return the language hooks for Python.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        enclosing_ancestors: &["function_definition", "class_definition"],
        reference_stoplist: &[
            "True", "False", "None", "self", "cls",
            "print", "open", "len", "range", "enumerate", "zip", "map", "filter",
            "str", "int", "float", "bool", "list", "dict", "tuple", "set",
            "type", "isinstance", "issubclass", "hasattr", "getattr", "setattr",
            "Exception", "ValueError", "TypeError", "KeyError",
        ],
    }
}
