use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Parser;

/// Extract exported symbols from Go source code.
/// Only symbols starting with an uppercase letter are exported in Go.
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_go::LANGUAGE.into()).is_err() {
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
            "function_declaration" => {
                if let Some(sym) = extract_function(source, file_path, &node) {
                    symbols.push(sym);
                }
            }
            "method_declaration" => {
                if let Some(sym) = extract_method(source, file_path, &node) {
                    symbols.push(sym);
                }
            }
            "type_declaration" => {
                extract_type_declarations(source, file_path, &node, &mut symbols);
            }
            _ => {}
        }
    }

    symbols
}

fn is_exported(name: &str) -> bool {
    name.chars().next().map_or(false, |c| c.is_uppercase())
}

fn extract_function(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
) -> Option<SymbolInfo> {
    let name = node
        .child_by_field_name("name")?
        .utf8_text(source.as_bytes())
        .ok()?
        .to_string();

    if !is_exported(&name) {
        return None;
    }

    let line = node.start_position().row + 1;
    let params = extract_parameters(source, node);
    let return_type = extract_return_type(source, node);
    let signature = build_signature(source, node);

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Function,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type,
        parameters: Some(params),
    })
}

fn extract_method(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
) -> Option<SymbolInfo> {
    let name = node
        .child_by_field_name("name")?
        .utf8_text(source.as_bytes())
        .ok()?
        .to_string();

    if !is_exported(&name) {
        return None;
    }

    // Extract receiver type
    let receiver = node
        .child_by_field_name("receiver")
        .and_then(|r| {
            // The receiver is a parameter_list, find the type inside
            for i in 0..r.child_count() {
                let child = r.child(i).unwrap();
                if child.kind() == "parameter_declaration" {
                    if let Some(type_node) = child.child_by_field_name("type") {
                        let type_text = type_node.utf8_text(source.as_bytes()).ok()?;
                        // Strip pointer: *AuthService -> AuthService
                        return Some(type_text.trim_start_matches('*').to_string());
                    }
                }
            }
            None
        });

    let line = node.start_position().row + 1;
    let params = extract_parameters(source, node);
    let return_type = extract_return_type(source, node);
    let signature = build_signature(source, node);

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Method,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: receiver,
        return_type,
        parameters: Some(params),
    })
}

fn extract_type_declarations(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    // type_declaration can contain one or more type_spec nodes
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "type_spec" {
            if let Some(sym) = extract_type_spec(source, file_path, &child) {
                symbols.push(sym);
            }
        }
    }
}

fn extract_type_spec(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
) -> Option<SymbolInfo> {
    let name = node
        .child_by_field_name("name")?
        .utf8_text(source.as_bytes())
        .ok()?
        .to_string();

    if !is_exported(&name) {
        return None;
    }

    let type_node = node.child_by_field_name("type")?;
    let kind = match type_node.kind() {
        "struct_type" => SymbolKind::Struct,
        "interface_type" => SymbolKind::Interface,
        _ => SymbolKind::Type,
    };

    let line = node.start_position().row + 1;
    let signature = format!("type {} {}", name, type_node.kind().replace("_type", ""));

    Some(SymbolInfo {
        name,
        kind,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    })
}

fn extract_parameters(source: &str, node: &tree_sitter::Node) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name("parameters") {
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

            // Go allows multiple names per parameter_declaration
            let name_node = child.child_by_field_name("name");
            if let Some(name) = name_node {
                params.push(Parameter {
                    name: name.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
                    type_annotation: type_ann,
                });
            }
        }
    }

    params
}

fn extract_return_type(source: &str, node: &tree_sitter::Node) -> Option<String> {
    node.child_by_field_name("result")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

fn build_signature(source: &str, node: &tree_sitter::Node) -> String {
    // Get text from `func` to end of result (or end of parameters)
    let start = node.start_byte();
    let end = node
        .child_by_field_name("result")
        .map(|n| n.end_byte())
        .or_else(|| node.child_by_field_name("parameters").map(|n| n.end_byte()))
        .unwrap_or(node.end_byte());

    let body_start = node.child_by_field_name("body").map(|n| n.start_byte());
    let actual_end = body_start.unwrap_or(end).min(end + 100);
    let actual_end = actual_end.max(end);

    source[start..actual_end.min(source.len())].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_exported_function() {
        let source = r#"package main

func ProcessPayment(amount float64, currency string) (*Receipt, error) {
    return nil, nil
}
"#;
        let symbols = extract(source, "handler.go");
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "ProcessPayment");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.signature.as_ref().unwrap().contains("ProcessPayment"));
        assert_eq!(sym.return_type.as_deref(), Some("(*Receipt, error)"));
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "amount");
        assert_eq!(params[0].type_annotation.as_deref(), Some("float64"));
    }

    #[test]
    fn test_extract_exported_struct() {
        let source = r#"package main

type AuthService struct {
    db *sql.DB
}
"#;
        let symbols = extract(source, "auth.go");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "AuthService");
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
    }

    #[test]
    fn test_extract_exported_interface() {
        let source = r#"package main

type Handler interface {
    ServeHTTP(w ResponseWriter, r *Request)
}
"#;
        let symbols = extract(source, "handler.go");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Handler");
        assert_eq!(symbols[0].kind, SymbolKind::Interface);
    }

    #[test]
    fn test_extract_method_with_receiver() {
        let source = r#"package main

func (s *AuthService) Validate(token string) error {
    return nil
}
"#;
        let symbols = extract(source, "auth.go");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Validate");
        assert_eq!(symbols[0].kind, SymbolKind::Method);
        assert_eq!(symbols[0].parent_symbol.as_deref(), Some("AuthService"));
    }

    #[test]
    fn test_skip_unexported() {
        let source = r#"package main

func internalHelper() {}
type internalType struct {}
"#;
        let symbols = extract(source, "internal.go");
        assert!(symbols.is_empty());
    }
}
