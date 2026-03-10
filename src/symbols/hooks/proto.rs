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

/// Build signature for proto symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Struct => {
            // message — check for parent
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
            // RPC — walk children for request/response types and streaming
            let mut message_types: Vec<(bool, String)> = Vec::new();
            let mut stream_next = false;

            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                match child.kind() {
                    "stream" => {
                        stream_next = true;
                    }
                    "message_or_enum_type" => {
                        let type_text = node_text(&child, source)
                            .unwrap_or("")
                            .to_string();
                        message_types.push((stream_next, type_text));
                        stream_next = false;
                    }
                    _ => {}
                }
            }

            let (req_stream, req_type) =
                message_types.first().cloned().unwrap_or((false, String::new()));
            let (resp_stream, resp_type) =
                message_types.get(1).cloned().unwrap_or((false, String::new()));

            let req_display = if req_stream {
                format!("stream {}", req_type)
            } else {
                req_type
            };
            let resp_display = if resp_stream {
                format!("stream {}", resp_type)
            } else {
                resp_type
            };

            format!("rpc {}({}) returns ({})", name, req_display, resp_display)
        }
        SymbolKind::Enum => {
            if let Some(parent) = resolve_parent(node, source) {
                format!("enum {}.{}", parent, name)
            } else {
                format!("enum {}", name)
            }
        }
        SymbolKind::Type => {
            // oneof — always has a parent message
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

    let mut message_types: Vec<(bool, String)> = Vec::new();
    let mut stream_next = false;

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "stream" => {
                stream_next = true;
            }
            "message_or_enum_type" => {
                let type_text = node_text(&child, source)
                    .unwrap_or("")
                    .to_string();
                message_types.push((stream_next, type_text));
                stream_next = false;
            }
            _ => {}
        }
    }

    if let Some((is_stream, type_name)) = message_types.first() {
        let param_type = if *is_stream {
            format!("stream {}", type_name)
        } else {
            type_name.clone()
        };
        vec![Parameter {
            name: "request".to_string(),
            type_annotation: Some(param_type),
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

    let mut message_types: Vec<(bool, String)> = Vec::new();
    let mut stream_next = false;

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "stream" => {
                stream_next = true;
            }
            "message_or_enum_type" => {
                let type_text = node_text(&child, source)
                    .unwrap_or("")
                    .to_string();
                message_types.push((stream_next, type_text));
                stream_next = false;
            }
            _ => {}
        }
    }

    message_types.get(1).map(|(is_stream, type_name)| {
        if *is_stream {
            format!("stream {}", type_name)
        } else {
            type_name.clone()
        }
    })
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
