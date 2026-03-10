use super::{find_ancestor, find_child_by_kind, field_text, LanguageHooks, Parameter, SymbolKind};
use tree_sitter::Node;

/// Check if a node has a `visibility_modifier` child (i.e., is `pub`).
fn is_visible(node: &Node, _source: &str) -> bool {
    find_child_by_kind(node, "visibility_modifier").is_some()
}

/// For methods inside an impl block, resolve the impl target type name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    // Walk up: function_item -> declaration_list -> impl_item
    let impl_node = find_ancestor(node, "impl_item")?;
    field_text(&impl_node, "type", source).map(|s| s.to_string())
}

/// Build a signature string for a Rust symbol.
///
/// For functions/methods: source span from node start to end of return_type (or parameters).
/// For structs/enums/traits: "pub struct Name", "pub enum Name", "pub trait Name".
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let start = node.start_byte();
            let end = node
                .child_by_field_name("return_type")
                .map(|n| n.end_byte())
                .or_else(|| node.child_by_field_name("parameters").map(|n| n.end_byte()))
                .unwrap_or(node.end_byte());

            let body_start = node.child_by_field_name("body").map(|n| n.start_byte());
            let actual_end = body_start.map_or(end, |bs| bs.min(end + 200));
            let actual_end = actual_end.max(end);

            source[start..actual_end.min(source.len())]
                .trim()
                .to_string()
        }
        SymbolKind::Struct => format!("pub struct {}", name),
        SymbolKind::Enum => format!("pub enum {}", name),
        SymbolKind::Trait => format!("pub trait {}", name),
        _ => format!("{:?} {}", kind, name),
    }
}

/// Extract parameters from a function/method node, skipping self parameters.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name("parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        match child.kind() {
            "parameter" => {
                let name = child
                    .child_by_field_name("pattern")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("")
                    .to_string();

                let type_ann = child
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.to_string());

                if !name.is_empty() && name != "self" && name != "&self" && name != "&mut self" {
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            "self_parameter" => {
                // Skip self/&self/&mut self
            }
            _ => {}
        }
    }

    params
}

/// Extract return type from a function/method node.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    field_text(node, "return_type", source).map(|s| s.to_string())
}

/// Return the language hooks for Rust.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: None,
    }
}
