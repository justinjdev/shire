use super::{find_ancestor, find_child_by_kind, node_text, LanguageHooks, Parameter, SymbolKind};
use tree_sitter::Node;

/// Proto visibility: all symbols are public.
fn is_visible(_node: &Node, _source: &str) -> bool {
    true
}

/// Resolve parent symbol for nested definitions.
/// - Messages/enums/oneofs inside a `message_body` get the parent message's `message_name`.
/// - RPCs inside a `service` get the parent service's `service_name`.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    let kind = node.kind();

    match kind {
        "rpc" => {
            let service = find_ancestor(node, "service")?;
            let name_node = find_child_by_kind(&service, "service_name")?;
            node_text(&name_node, source).map(|s| s.to_string())
        }
        "message" | "enum" | "oneof" => {
            // Check if inside a message_body (i.e., nested)
            let body = find_ancestor(node, "message_body")?;
            let parent_msg = body.parent()?;
            if parent_msg.kind() != "message" {
                return None;
            }
            let name_node = find_child_by_kind(&parent_msg, "message_name")?;
            node_text(&name_node, source).map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Extract RPC message types with streaming flags from an RPC node.
/// Returns a vector of (is_stream, type_name) tuples.
fn extract_rpc_message_types(node: &Node, source: &str) -> Vec<(bool, String)> {
    let mut message_types = Vec::new();
    let mut stream_next = false;

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "stream" => stream_next = true,
            "message_or_enum_type" => {
                let type_text = node_text(&child, source).unwrap_or("").to_string();
                message_types.push((stream_next, type_text));
                stream_next = false;
            }
            _ => {}
        }
    }
    message_types
}

/// Format a message type with optional streaming prefix.
fn format_message_type(is_stream: bool, type_name: &str) -> String {
    if is_stream {
        format!("stream {}", type_name)
    } else {
        type_name.to_string()
    }
}

/// Build signature for proto symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Struct => {
            if let Some(parent) = resolve_parent(node, source) {
                format!("message {}.{}", parent, name)
            } else {
                format!("message {}", name)
            }
        }
        SymbolKind::Interface => {
            format!("service {}", name)
        }
        SymbolKind::Method => {
            let types = extract_rpc_message_types(node, source);
            let (req_stream, req_type) = types.first().cloned().unwrap_or((false, String::new()));
            let (resp_stream, resp_type) = types.get(1).cloned().unwrap_or((false, String::new()));
            format!(
                "rpc {}({}) returns ({})",
                name,
                format_message_type(req_stream, &req_type),
                format_message_type(resp_stream, &resp_type)
            )
        }
        SymbolKind::Enum => {
            if let Some(parent) = resolve_parent(node, source) {
                format!("enum {}.{}", parent, name)
            } else {
                format!("enum {}", name)
            }
        }
        SymbolKind::Type => {
            if let Some(parent) = resolve_parent(node, source) {
                format!("oneof {}.{}", parent, name)
            } else {
                format!("oneof {}", name)
            }
        }
        _ => format!("{:?} {}", kind, name),
    }
}

/// Extract parameters for RPC methods.
/// Returns a single "request" parameter with the request type.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    if node.kind() != "rpc" {
        return Vec::new();
    }

    let types = extract_rpc_message_types(node, source);
    if let Some((is_stream, type_name)) = types.first() {
        vec![Parameter {
            name: "request".to_string(),
            type_annotation: Some(format_message_type(*is_stream, type_name)),
        }]
    } else {
        Vec::new()
    }
}

/// Extract return type for RPC methods.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "rpc" {
        return None;
    }

    let types = extract_rpc_message_types(node, source);
    types
        .get(1)
        .map(|(is_stream, type_name)| format_message_type(*is_stream, type_name))
}

/// Return Protobuf language hooks.
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
