use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Parser;

/// Extract symbols from Protocol Buffer source files.
/// Handles messages (with nesting), services, RPCs, enums, and oneofs.
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_proto::LANGUAGE.into()).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut symbols = Vec::new();
    let root = tree.root_node();

    for i in 0..root.child_count() {
        let node = root.child(i).unwrap();
        match node.kind() {
            "message" => extract_message(source, file_path, &node, None, &mut symbols),
            "service" => extract_service(source, file_path, &node, &mut symbols),
            "enum" => {
                if let Some(sym) = extract_enum(source, file_path, &node, None) {
                    symbols.push(sym);
                }
            }
            _ => {}
        }
    }

    symbols
}

/// Extract a message definition and recursively process nested messages, enums, and oneofs.
fn extract_message(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    parent: Option<&str>,
    symbols: &mut Vec<SymbolInfo>,
) {
    let name = match find_child_text(node, "message_name", source) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row + 1;
    let signature = match parent {
        Some(p) => format!("message {}.{}", p, name),
        None => format!("message {}", name),
    };

    symbols.push(SymbolInfo {
        name: name.clone(),
        kind: SymbolKind::Struct,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: parent.map(|s| s.to_string()),
        return_type: None,
        parameters: None,
    });

    // Walk the message_body for nested definitions
    if let Some(body) = find_child_by_kind(node, "message_body") {
        for i in 0..body.child_count() {
            let child = body.child(i).unwrap();
            match child.kind() {
                "message" => extract_message(source, file_path, &child, Some(&name), symbols),
                "enum" => {
                    if let Some(sym) = extract_enum(source, file_path, &child, Some(&name)) {
                        symbols.push(sym);
                    }
                }
                "oneof" => {
                    if let Some(sym) = extract_oneof(source, file_path, &child, &name) {
                        symbols.push(sym);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Extract a service definition and its RPC methods.
fn extract_service(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    let name = match find_child_text(node, "service_name", source) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row + 1;
    let signature = format!("service {}", name);

    symbols.push(SymbolInfo {
        name: name.clone(),
        kind: SymbolKind::Interface,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });

    // Extract RPCs within the service
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "rpc" {
            if let Some(sym) = extract_rpc(source, file_path, &child, &name) {
                symbols.push(sym);
            }
        }
    }
}

/// Extract an RPC method definition, including streaming annotations.
fn extract_rpc(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    service_name: &str,
) -> Option<SymbolInfo> {
    let rpc_name = find_child_text(node, "rpc_name", source)?;
    let line = node.start_position().row + 1;

    // Parse the RPC signature to extract request/response types and streaming flags.
    // Grammar structure: rpc rpc_name ( [stream] message_or_enum_type ) returns ( [stream] message_or_enum_type )
    let mut message_types: Vec<(bool, String)> = Vec::new(); // (is_stream, type_name)
    let mut stream_next = false;

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "stream" => {
                stream_next = true;
            }
            "message_or_enum_type" => {
                let type_text = child
                    .utf8_text(source.as_bytes())
                    .ok()?
                    .to_string();
                message_types.push((stream_next, type_text));
                stream_next = false;
            }
            _ => {}
        }
    }

    let (request_stream, request_type) = message_types.first().cloned().unwrap_or((false, String::new()));
    let (response_stream, response_type) = message_types.get(1).cloned().unwrap_or((false, String::new()));

    let req_display = if request_stream {
        format!("stream {}", request_type)
    } else {
        request_type.clone()
    };
    let resp_display = if response_stream {
        format!("stream {}", response_type)
    } else {
        response_type.clone()
    };

    let signature = format!(
        "rpc {}({}) returns ({})",
        rpc_name, req_display, resp_display
    );

    let param_type = if request_stream {
        format!("stream {}", request_type)
    } else {
        request_type
    };

    let return_type = if response_stream {
        format!("stream {}", response_type)
    } else {
        response_type
    };

    Some(SymbolInfo {
        name: rpc_name,
        kind: SymbolKind::Method,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: Some(service_name.to_string()),
        return_type: Some(return_type),
        parameters: Some(vec![Parameter {
            name: "request".to_string(),
            type_annotation: Some(param_type),
        }]),
    })
}

/// Extract an enum definition.
fn extract_enum(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    parent: Option<&str>,
) -> Option<SymbolInfo> {
    let name = find_child_text(node, "enum_name", source)?;
    let line = node.start_position().row + 1;
    let signature = match parent {
        Some(p) => format!("enum {}.{}", p, name),
        None => format!("enum {}", name),
    };

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Enum,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: parent.map(|s| s.to_string()),
        return_type: None,
        parameters: None,
    })
}

/// Extract a oneof definition within a message.
fn extract_oneof(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    message_name: &str,
) -> Option<SymbolInfo> {
    // oneof uses a bare `identifier` child for its name (not a typed name node)
    let name = find_child_text(node, "identifier", source)?;
    let line = node.start_position().row + 1;
    let signature = format!("oneof {}.{}", message_name, name);

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Type,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: Some(message_name.to_string()),
        return_type: None,
        parameters: None,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the first child with the given node kind and return its text content.
fn find_child_text(node: &tree_sitter::Node, kind: &str, source: &str) -> Option<String> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == kind {
            return child.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
        }
    }
    None
}

/// Find the first child with the given node kind.
fn find_child_by_kind<'a>(node: &'a tree_sitter::Node, kind: &str) -> Option<tree_sitter::Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_top_level_message() {
        let source = r#"syntax = "proto3";

message SearchRequest {
  string query = 1;
  int32 page_number = 2;
}
"#;
        let symbols = extract(source, "search.proto");
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "SearchRequest");
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert_eq!(sym.visibility, "public");
        assert!(sym.parent_symbol.is_none());
        assert_eq!(sym.signature.as_deref(), Some("message SearchRequest"));
        assert_eq!(sym.file_path, "search.proto");
        assert_eq!(sym.line, 3);
    }

    #[test]
    fn test_extract_service_and_rpc() {
        let source = r#"syntax = "proto3";

service SearchService {
  rpc Search (SearchRequest) returns (SearchResponse);
}
"#;
        let symbols = extract(source, "search.proto");
        assert_eq!(symbols.len(), 2);

        let svc = &symbols[0];
        assert_eq!(svc.name, "SearchService");
        assert_eq!(svc.kind, SymbolKind::Interface);
        assert_eq!(svc.signature.as_deref(), Some("service SearchService"));

        let rpc = &symbols[1];
        assert_eq!(rpc.name, "Search");
        assert_eq!(rpc.kind, SymbolKind::Method);
        assert_eq!(rpc.parent_symbol.as_deref(), Some("SearchService"));
        assert_eq!(rpc.return_type.as_deref(), Some("SearchResponse"));
        let params = rpc.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "request");
        assert_eq!(params[0].type_annotation.as_deref(), Some("SearchRequest"));
        assert_eq!(
            rpc.signature.as_deref(),
            Some("rpc Search(SearchRequest) returns (SearchResponse)")
        );
    }

    #[test]
    fn test_extract_streaming_rpc() {
        let source = r#"syntax = "proto3";

service StreamService {
  rpc ClientStream (stream UpdateRequest) returns (UpdateResponse);
  rpc ServerStream (GetRequest) returns (stream GetResponse);
  rpc BiDiStream (stream ChatMessage) returns (stream ChatMessage);
}
"#;
        let symbols = extract(source, "stream.proto");
        // 1 service + 3 rpcs
        assert_eq!(symbols.len(), 4);

        // Client streaming
        let client_rpc = &symbols[1];
        assert_eq!(client_rpc.name, "ClientStream");
        let params = client_rpc.parameters.as_ref().unwrap();
        assert_eq!(
            params[0].type_annotation.as_deref(),
            Some("stream UpdateRequest")
        );
        assert_eq!(client_rpc.return_type.as_deref(), Some("UpdateResponse"));

        // Server streaming
        let server_rpc = &symbols[2];
        assert_eq!(server_rpc.name, "ServerStream");
        let params = server_rpc.parameters.as_ref().unwrap();
        assert_eq!(params[0].type_annotation.as_deref(), Some("GetRequest"));
        assert_eq!(
            server_rpc.return_type.as_deref(),
            Some("stream GetResponse")
        );

        // Bidirectional streaming
        let bidi_rpc = &symbols[3];
        assert_eq!(bidi_rpc.name, "BiDiStream");
        let params = bidi_rpc.parameters.as_ref().unwrap();
        assert_eq!(
            params[0].type_annotation.as_deref(),
            Some("stream ChatMessage")
        );
        assert_eq!(
            bidi_rpc.return_type.as_deref(),
            Some("stream ChatMessage")
        );
    }

    #[test]
    fn test_extract_enum() {
        let source = r#"syntax = "proto3";

enum Status {
  UNKNOWN = 0;
  ACTIVE = 1;
  INACTIVE = 2;
}
"#;
        let symbols = extract(source, "status.proto");
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "Status");
        assert_eq!(sym.kind, SymbolKind::Enum);
        assert!(sym.parent_symbol.is_none());
        assert_eq!(sym.signature.as_deref(), Some("enum Status"));
    }

    #[test]
    fn test_extract_nested_message_and_enum() {
        let source = r#"syntax = "proto3";

message Outer {
  string id = 1;

  message Inner {
    int32 value = 1;
  }

  enum Color {
    RED = 0;
    BLUE = 1;
  }
}
"#;
        let symbols = extract(source, "nested.proto");
        assert_eq!(symbols.len(), 3);

        let outer = &symbols[0];
        assert_eq!(outer.name, "Outer");
        assert_eq!(outer.kind, SymbolKind::Struct);
        assert!(outer.parent_symbol.is_none());

        let inner = &symbols[1];
        assert_eq!(inner.name, "Inner");
        assert_eq!(inner.kind, SymbolKind::Struct);
        assert_eq!(inner.parent_symbol.as_deref(), Some("Outer"));
        assert_eq!(inner.signature.as_deref(), Some("message Outer.Inner"));

        let color = &symbols[2];
        assert_eq!(color.name, "Color");
        assert_eq!(color.kind, SymbolKind::Enum);
        assert_eq!(color.parent_symbol.as_deref(), Some("Outer"));
        assert_eq!(color.signature.as_deref(), Some("enum Outer.Color"));
    }

    #[test]
    fn test_extract_oneof() {
        let source = r#"syntax = "proto3";

message SampleMessage {
  oneof test_oneof {
    string name = 4;
    int32 id = 5;
  }
}
"#;
        let symbols = extract(source, "oneof.proto");
        assert_eq!(symbols.len(), 2);

        let msg = &symbols[0];
        assert_eq!(msg.name, "SampleMessage");

        let oneof = &symbols[1];
        assert_eq!(oneof.name, "test_oneof");
        assert_eq!(oneof.kind, SymbolKind::Type);
        assert_eq!(oneof.parent_symbol.as_deref(), Some("SampleMessage"));
        assert_eq!(
            oneof.signature.as_deref(),
            Some("oneof SampleMessage.test_oneof")
        );
    }

    #[test]
    fn test_empty_file() {
        let symbols = extract("", "empty.proto");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_unparseable_file() {
        let symbols = extract("this is not valid protobuf {{{{", "bad.proto");
        // Should not panic; may produce no symbols or partial results
        // The key requirement is resilience
        let _ = symbols;
    }

    #[test]
    fn test_syntax_only_file() {
        let source = r#"syntax = "proto3";
package example.v1;
"#;
        let symbols = extract(source, "empty_pkg.proto");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_multiple_services_and_messages() {
        let source = r#"syntax = "proto3";

message Request {
  string data = 1;
}

message Response {
  string result = 1;
}

service Alpha {
  rpc DoAlpha (Request) returns (Response);
}

service Beta {
  rpc DoBeta (Request) returns (Response);
}
"#;
        let symbols = extract(source, "multi.proto");
        // 2 messages + 2 services + 2 rpcs = 6
        assert_eq!(symbols.len(), 6);

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Request"));
        assert!(names.contains(&"Response"));
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
        assert!(names.contains(&"DoAlpha"));
        assert!(names.contains(&"DoBeta"));
    }
}
