use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Parser;

/// Extract public symbols from Python source code.
/// All top-level functions and classes are considered public.
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .is_err()
    {
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
            "function_definition" => {
                if let Some(sym) = extract_function(source, file_path, &node) {
                    symbols.push(sym);
                }
            }
            "class_definition" => {
                extract_class(source, file_path, &node, &mut symbols);
            }
            _ => {}
        }
    }

    symbols
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

    let line = node.start_position().row + 1;
    let params = extract_parameters(source, node);
    let return_type = extract_return_type(source, node);
    let signature = build_signature(source, node, &name);

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

fn extract_class(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    let class_name = match node.child_by_field_name("name") {
        Some(n) => match n.utf8_text(source.as_bytes()) {
            Ok(s) => s.to_string(),
            Err(_) => return,
        },
        None => return,
    };

    let line = node.start_position().row + 1;
    let signature = format!("class {}", class_name);

    symbols.push(SymbolInfo {
        name: class_name.clone(),
        kind: SymbolKind::Class,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });

    // Extract methods from class body
    if let Some(body) = node.child_by_field_name("body") {
        for i in 0..body.child_count() {
            let child = body.child(i).unwrap();
            if child.kind() == "function_definition" {
                if let Some(method_name_node) = child.child_by_field_name("name") {
                    if let Ok(method_name) = method_name_node.utf8_text(source.as_bytes()) {
                        // __init__ is extracted for constructor signature
                        // Skip other dunder and _private methods
                        if method_name == "__init__" {
                            if let Some(mut sym) = extract_function(source, file_path, &child) {
                                sym.kind = SymbolKind::Method;
                                sym.parent_symbol = Some(class_name.clone());
                                // Filter out 'self' from params
                                sym.parameters = sym.parameters.map(|params| {
                                    params
                                        .into_iter()
                                        .filter(|p| p.name != "self")
                                        .collect()
                                });
                                symbols.push(sym);
                            }
                        } else if method_name.starts_with('_') {
                            continue;
                        } else {
                            if let Some(mut sym) = extract_function(source, file_path, &child) {
                                sym.kind = SymbolKind::Method;
                                sym.parent_symbol = Some(class_name.clone());
                                // Filter out 'self' from params
                                sym.parameters = sym.parameters.map(|params| {
                                    params
                                        .into_iter()
                                        .filter(|p| p.name != "self")
                                        .collect()
                                });
                                symbols.push(sym);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn extract_parameters(source: &str, node: &tree_sitter::Node) -> Vec<Parameter> {
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

fn extract_return_type(source: &str, node: &tree_sitter::Node) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

fn build_signature(source: &str, node: &tree_sitter::Node, name: &str) -> String {
    let params_text = node
        .child_by_field_name("parameters")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .unwrap_or("()");
    let ret = extract_return_type(source, node)
        .map(|r| format!(" -> {}", r))
        .unwrap_or_default();
    format!("def {}{}{}", name, params_text, ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function_with_type_hints() {
        let source = r#"def process_payment(amount: float, currency: str) -> Receipt:
    pass
"#;
        let symbols = extract(source, "pay.py");
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "process_payment");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.return_type.as_deref(), Some("Receipt"));
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "amount");
        assert_eq!(params[0].type_annotation.as_deref(), Some("float"));
        assert_eq!(params[1].name, "currency");
        assert_eq!(params[1].type_annotation.as_deref(), Some("str"));
    }

    #[test]
    fn test_extract_class_with_methods() {
        let source = r#"class AuthService:
    def __init__(self, db: Database):
        self.db = db

    def validate(self, token: str) -> bool:
        return True

    def _internal(self):
        pass
"#;
        let symbols = extract(source, "auth.py");
        // class + __init__ + validate (skip _internal)
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "AuthService");
        assert_eq!(symbols[0].kind, SymbolKind::Class);

        assert_eq!(symbols[1].name, "__init__");
        assert_eq!(symbols[1].kind, SymbolKind::Method);
        assert_eq!(symbols[1].parent_symbol.as_deref(), Some("AuthService"));
        // self should be filtered out
        let init_params = symbols[1].parameters.as_ref().unwrap();
        assert_eq!(init_params.len(), 1);
        assert_eq!(init_params[0].name, "db");

        assert_eq!(symbols[2].name, "validate");
        assert_eq!(symbols[2].kind, SymbolKind::Method);

        assert!(!symbols.iter().any(|s| s.name == "_internal"));
    }

    #[test]
    fn test_extract_function_no_hints() {
        let source = r#"def greet(name):
    return f"Hello {name}"
"#;
        let symbols = extract(source, "greet.py");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert!(symbols[0].return_type.is_none());
    }
}
