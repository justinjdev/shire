use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Parser;

/// Extract public symbols from Rust source code.
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_rust::LANGUAGE.into()).is_err() {
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
            "function_item" => {
                if is_pub(&node) {
                    if let Some(sym) = extract_function(source, file_path, &node) {
                        symbols.push(sym);
                    }
                }
            }
            "struct_item" => {
                if is_pub(&node) {
                    if let Some(sym) = extract_struct(source, file_path, &node) {
                        symbols.push(sym);
                    }
                }
            }
            "enum_item" => {
                if is_pub(&node) {
                    if let Some(sym) = extract_enum(source, file_path, &node) {
                        symbols.push(sym);
                    }
                }
            }
            "trait_item" => {
                if is_pub(&node) {
                    if let Some(sym) = extract_trait(source, file_path, &node) {
                        symbols.push(sym);
                    }
                }
            }
            "impl_item" => {
                extract_impl_methods(source, file_path, &node, &mut symbols);
            }
            _ => {}
        }
    }

    symbols
}

fn is_pub(node: &tree_sitter::Node) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "visibility_modifier" {
            return true;
        }
    }
    false
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

fn extract_struct(
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

    Some(SymbolInfo {
        name: name.clone(),
        kind: SymbolKind::Struct,
        signature: Some(format!("pub struct {}", name)),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    })
}

fn extract_enum(
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

    Some(SymbolInfo {
        name: name.clone(),
        kind: SymbolKind::Enum,
        signature: Some(format!("pub enum {}", name)),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    })
}

fn extract_trait(
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

    Some(SymbolInfo {
        name: name.clone(),
        kind: SymbolKind::Trait,
        signature: Some(format!("pub trait {}", name)),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    })
}

fn extract_impl_methods(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    // Get the impl target type
    let impl_type = node
        .child_by_field_name("type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string());

    // Find the body (declaration_list)
    if let Some(body) = node.child_by_field_name("body") {
        for i in 0..body.child_count() {
            let child = body.child(i).unwrap();
            if child.kind() == "function_item" && is_pub(&child) {
                if let Some(mut sym) = extract_function(source, file_path, &child) {
                    sym.kind = SymbolKind::Method;
                    sym.parent_symbol = impl_type.clone();
                    symbols.push(sym);
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

fn extract_return_type(source: &str, node: &tree_sitter::Node) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

fn build_signature(source: &str, node: &tree_sitter::Node) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pub_function() {
        let source = r#"pub fn process_payment(amount: f64, currency: &str) -> Result<Receipt> {
    todo!()
}"#;
        let symbols = extract(source, "src/pay.rs");
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "process_payment");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.signature.as_ref().unwrap().contains("pub fn process_payment"));
        assert_eq!(sym.return_type.as_deref(), Some("Result<Receipt>"));
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "amount");
        assert_eq!(params[0].type_annotation.as_deref(), Some("f64"));
        assert_eq!(params[1].name, "currency");
        assert_eq!(params[1].type_annotation.as_deref(), Some("&str"));
    }

    #[test]
    fn test_extract_pub_struct() {
        let source = r#"pub struct AuthService {
    db: Connection,
}"#;
        let symbols = extract(source, "src/auth.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "AuthService");
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
    }

    #[test]
    fn test_extract_pub_enum() {
        let source = r#"pub enum Status {
    Active,
    Inactive,
}"#;
        let symbols = extract(source, "src/types.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Status");
        assert_eq!(symbols[0].kind, SymbolKind::Enum);
    }

    #[test]
    fn test_extract_pub_trait() {
        let source = r#"pub trait Handler {
    fn handle(&self) -> Result<()>;
}"#;
        let symbols = extract(source, "src/handler.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Handler");
        assert_eq!(symbols[0].kind, SymbolKind::Trait);
    }

    #[test]
    fn test_extract_impl_method() {
        let source = r#"impl AuthService {
    pub fn validate(&self, token: &str) -> Result<()> {
        todo!()
    }

    fn internal_helper(&self) {}
}"#;
        let symbols = extract(source, "src/auth.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "validate");
        assert_eq!(symbols[0].kind, SymbolKind::Method);
        assert_eq!(symbols[0].parent_symbol.as_deref(), Some("AuthService"));
        let params = symbols[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1); // self is skipped
        assert_eq!(params[0].name, "token");
    }

    #[test]
    fn test_skip_non_pub() {
        let source = r#"fn internal_fn() {}
struct InternalStruct {}
enum InternalEnum {}
"#;
        let symbols = extract(source, "src/internal.rs");
        assert!(symbols.is_empty());
    }
}
