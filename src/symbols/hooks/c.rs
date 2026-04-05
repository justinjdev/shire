use super::{find_child_by_kind, node_text, LanguageHooks, Parameter, SymbolKind};
use tree_sitter::Node;

/// C visibility: skip `static` functions/symbols (file-local linkage).
/// All other symbols are included.
fn is_visible(node: &Node, source: &str) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "storage_class_specifier" {
            if let Some(text) = node_text(&child, source) {
                if text == "static" {
                    return false;
                }
            }
        }
    }
    true
}

/// C has no classes or methods — no parent resolution needed.
fn resolve_parent(_node: &Node, _source: &str) -> Option<String> {
    None
}

/// Build signature for C symbols.
/// For functions: source from node start to the body opening brace.
/// For structs/enums: "struct Name" or "enum Name".
/// For type aliases: the full typedef text.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Struct => format!("struct {}", name),
        SymbolKind::Enum => format!("enum {}", name),
        SymbolKind::Type => {
            // For typedefs, extract the full typedef text
            node_text(node, source)
                .map(|s| s.trim().trim_end_matches(';').trim().to_string())
                .unwrap_or_else(|| format!("typedef {}", name))
        }
        _ => {
            // function_definition: extract from start to body
            let start = node.start_byte();
            let end = node
                .child_by_field_name("body")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());

            source[start..end.min(source.len())].trim().to_string()
        }
    }
}

/// Extract parameters from C function definitions.
/// The parameter_list is a child of the function_declarator inside the declarator field.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let declarator = match node.child_by_field_name("declarator") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let params_node = match find_child_by_kind(&declarator, "parameter_list") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter_declaration" {
            let type_ann = child
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());

            let param_name = child
                .child_by_field_name("declarator")
                .and_then(|n| {
                    // The declarator might be an identifier or a pointer_declarator
                    if n.kind() == "identifier" {
                        n.utf8_text(source.as_bytes()).ok()
                    } else {
                        // For pointer declarators, find the nested identifier
                        find_child_by_kind(&n, "identifier")
                            .and_then(|id| id.utf8_text(source.as_bytes()).ok())
                    }
                })
                .unwrap_or("");

            if !param_name.is_empty() {
                params.push(Parameter {
                    name: param_name.to_string(),
                    type_annotation: type_ann,
                });
            }
        }
    }

    params
}

/// Extract return type from the `type` field of function_definition.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Return C language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: None,
        enclosing_ancestors: &[],
        reference_stoplist: &[],
    }
}
