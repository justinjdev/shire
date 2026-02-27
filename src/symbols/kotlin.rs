use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Parser;

/// Extract public symbols from Kotlin source code.
/// Kotlin defaults to public visibility; symbols with `private` or `internal`
/// modifiers are skipped.
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
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
            "class_declaration" => {
                if !is_private_or_internal(source, &node) {
                    extract_class_declaration(source, file_path, &node, &mut symbols);
                }
            }
            "object_declaration" => {
                if !is_private_or_internal(source, &node) {
                    extract_object_declaration(source, file_path, &node, &mut symbols);
                }
            }
            "function_declaration" => {
                if !is_private_or_internal(source, &node) {
                    if let Some(sym) = extract_function(source, file_path, &node, None) {
                        symbols.push(sym);
                    }
                }
            }
            _ => {}
        }
    }

    symbols
}

/// Check whether a node has `private` or `internal` visibility modifiers.
fn is_private_or_internal(source: &str, node: &tree_sitter::Node) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "modifiers" {
            for j in 0..child.child_count() {
                let modifier = child.child(j).unwrap();
                if modifier.kind() == "visibility_modifier" {
                    if let Ok(text) = modifier.utf8_text(source.as_bytes()) {
                        if text == "private" || text == "internal" {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Determine what kind of class_declaration this is (class, interface, or enum class)
/// and extract the symbol plus any public methods inside its body.
fn extract_class_declaration(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    let name = match find_identifier(source, node) {
        Some(n) => n,
        None => return,
    };
    let line = node.start_position().row + 1;

    // Determine if this is class, interface, or enum class by scanning keyword children.
    let mut kind = SymbolKind::Class;
    let mut keyword = "class";

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if let Ok(text) = child.utf8_text(source.as_bytes()) {
            match text {
                "interface" => {
                    kind = SymbolKind::Interface;
                    keyword = "interface";
                    break;
                }
                "enum" => {
                    kind = SymbolKind::Enum;
                    keyword = "enum class";
                }
                "class" if kind == SymbolKind::Enum => {
                    break;
                }
                "class" => {
                    kind = SymbolKind::Class;
                    keyword = "class";
                    break;
                }
                _ => {}
            }
        }
    }

    let signature = format!("{} {}", keyword, name);

    symbols.push(SymbolInfo {
        name: name.clone(),
        kind,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });

    // Extract public methods from the class body
    extract_methods_from_body(source, file_path, node, &name, symbols);
}

/// Extract an object_declaration as a Class symbol, plus any public methods inside it.
fn extract_object_declaration(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    let name = match find_identifier(source, node) {
        Some(n) => n,
        None => return,
    };
    let line = node.start_position().row + 1;
    let signature = format!("object {}", name);

    symbols.push(SymbolInfo {
        name: name.clone(),
        kind: SymbolKind::Class,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });

    // Extract public methods from the object body
    extract_methods_from_body(source, file_path, node, &name, symbols);
}

/// Extract a function_declaration. If parent_name is Some, this is a method.
fn extract_function(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    parent_name: Option<&str>,
) -> Option<SymbolInfo> {
    let name = find_identifier(source, node)?;
    let line = node.start_position().row + 1;
    let params = extract_parameters(source, node);
    let return_type = extract_return_type(source, node);
    let signature = build_signature(source, node);

    let (kind, parent_symbol) = match parent_name {
        Some(p) => (SymbolKind::Method, Some(p.to_string())),
        None => (SymbolKind::Function, None),
    };

    Some(SymbolInfo {
        name,
        kind,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol,
        return_type,
        parameters: Some(params),
    })
}

/// Find methods inside a class/object body and add them to the symbols list.
fn extract_methods_from_body(
    source: &str,
    file_path: &str,
    parent_node: &tree_sitter::Node,
    parent_name: &str,
    symbols: &mut Vec<SymbolInfo>,
) {
    for i in 0..parent_node.child_count() {
        let child = parent_node.child(i).unwrap();
        if child.kind() == "class_body" || child.kind() == "enum_class_body" {
            for j in 0..child.child_count() {
                let member = child.child(j).unwrap();
                if member.kind() == "function_declaration"
                    && !is_private_or_internal(source, &member)
                {
                    if let Some(sym) =
                        extract_function(source, file_path, &member, Some(parent_name))
                    {
                        symbols.push(sym);
                    }
                }
            }
        }
    }
}

/// Find the identifier child of a node (used for class/object/function names).
/// Tries `identifier`, `type_identifier`, and `simple_identifier` to handle
/// different tree-sitter-kotlin grammar versions.
fn find_identifier(source: &str, node: &tree_sitter::Node) -> Option<String> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "identifier" | "type_identifier" | "simple_identifier" => {
                return child
                    .utf8_text(source.as_bytes())
                    .ok()
                    .map(|s| s.to_string());
            }
            _ => {}
        }
    }
    None
}

/// Extract parameters from `function_value_parameters` child.
fn extract_parameters(source: &str, node: &tree_sitter::Node) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "function_value_parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter" {
            let name = find_identifier(source, &child).unwrap_or_default();
            let type_ann = extract_parameter_type(source, &child);

            if !name.is_empty() {
                params.push(Parameter {
                    name,
                    type_annotation: type_ann,
                });
            }
        }
    }

    params
}

/// Extract the type annotation from a `parameter` node.
/// A parameter is structured as: simple_identifier ":" type
fn extract_parameter_type(source: &str, param_node: &tree_sitter::Node) -> Option<String> {
    let mut found_colon = false;
    for i in 0..param_node.child_count() {
        let child = param_node.child(i).unwrap();
        if child.kind() == ":" {
            found_colon = true;
            continue;
        }
        if found_colon {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Extract the return type from a function_declaration.
/// The return type follows a ":" after the function_value_parameters.
fn extract_return_type(source: &str, node: &tree_sitter::Node) -> Option<String> {
    let mut after_params = false;
    let mut found_colon = false;

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();

        if child.kind() == "function_value_parameters" {
            after_params = true;
            continue;
        }

        if after_params && child.kind() == ":" {
            found_colon = true;
            continue;
        }

        if found_colon {
            // Skip if we hit the body or constraints before a type
            if child.kind() == "function_body" || child.kind() == "type_constraints" {
                return None;
            }
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }

    None
}

/// Build a signature string for a function_declaration.
/// Captures from the start of the node up to (but not including) the function body.
fn build_signature(source: &str, node: &tree_sitter::Node) -> String {
    let start = node.start_byte();

    // Find where the body starts and exclude it
    let body_start = find_child_by_kind(node, "function_body").map(|n| n.start_byte());

    let end = match body_start {
        Some(bs) => bs,
        None => node.end_byte(),
    };

    source[start..end.min(source.len())].trim().to_string()
}

/// Find the first child of a node with the given kind.
fn find_child_by_kind<'a>(
    node: &'a tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
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
    fn test_extract_class() {
        let source = r#"class UserService {
    fun validate(token: String): Boolean {
        return true
    }
}"#;
        let symbols = extract(source, "UserService.kt");
        let class_sym = symbols.iter().find(|s| s.name == "UserService").unwrap();
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.line, 1);
        assert_eq!(class_sym.visibility, "public");
        assert!(class_sym
            .signature
            .as_ref()
            .unwrap()
            .contains("class UserService"));
    }

    #[test]
    fn test_extract_interface() {
        let source = r#"interface Repository {
    fun findById(id: String): Entity?
}"#;
        let symbols = extract(source, "Repository.kt");
        let iface = symbols
            .iter()
            .find(|s| s.name == "Repository")
            .expect("should find Repository");
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert!(iface
            .signature
            .as_ref()
            .unwrap()
            .contains("interface Repository"));
    }

    #[test]
    fn test_extract_object_declaration() {
        let source = r#"object DatabaseConfig {
    val url = "jdbc:postgresql://localhost/db"
}"#;
        let symbols = extract(source, "Config.kt");
        let obj = symbols
            .iter()
            .find(|s| s.name == "DatabaseConfig")
            .expect("should find DatabaseConfig");
        assert_eq!(obj.kind, SymbolKind::Class);
        assert!(obj
            .signature
            .as_ref()
            .unwrap()
            .contains("object DatabaseConfig"));
    }

    #[test]
    fn test_extract_enum_class() {
        let source = r#"enum class Status {
    ACTIVE,
    INACTIVE,
    SUSPENDED
}"#;
        let symbols = extract(source, "Status.kt");
        let enum_sym = symbols
            .iter()
            .find(|s| s.name == "Status")
            .expect("should find Status");
        assert_eq!(enum_sym.kind, SymbolKind::Enum);
        assert!(enum_sym
            .signature
            .as_ref()
            .unwrap()
            .contains("enum class Status"));
    }

    #[test]
    fn test_extract_top_level_function() {
        let source = r#"fun processPayment(amount: Double, currency: String): Receipt {
    return Receipt()
}"#;
        let symbols = extract(source, "Payment.kt");
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "processPayment");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym
            .signature
            .as_ref()
            .unwrap()
            .contains("fun processPayment"));
        assert_eq!(sym.return_type.as_deref(), Some("Receipt"));
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "amount");
        assert_eq!(params[0].type_annotation.as_deref(), Some("Double"));
        assert_eq!(params[1].name, "currency");
        assert_eq!(params[1].type_annotation.as_deref(), Some("String"));
    }

    #[test]
    fn test_extract_class_method() {
        let source = r#"class AuthService {
    fun validate(token: String): Boolean {
        return true
    }
}"#;
        let symbols = extract(source, "AuthService.kt");
        let method = symbols
            .iter()
            .find(|s| s.name == "validate")
            .expect("should find validate method");
        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.parent_symbol.as_deref(), Some("AuthService"));
        let params = method.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "token");
        assert_eq!(params[0].type_annotation.as_deref(), Some("String"));
        assert_eq!(method.return_type.as_deref(), Some("Boolean"));
    }

    #[test]
    fn test_skip_private_class() {
        let source = r#"private class InternalHelper {
    fun doSomething() {}
}"#;
        let symbols = extract(source, "Internal.kt");
        assert!(
            symbols.is_empty(),
            "private class and its methods should be skipped, got: {:?}",
            symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_skip_internal_function() {
        let source = r#"internal fun helperFunction(x: Int): Int {
    return x * 2
}"#;
        let symbols = extract(source, "Helper.kt");
        assert!(
            symbols.is_empty(),
            "internal function should be skipped, got: {:?}",
            symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_public_modifier_explicit() {
        let source = r#"public fun explicitPublic(name: String): String {
    return name
}"#;
        let symbols = extract(source, "Public.kt");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "explicitPublic");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_skip_private_method_in_public_class() {
        let source = r#"class PublicService {
    fun publicMethod(): String {
        return ""
    }

    private fun secretMethod(): String {
        return ""
    }
}"#;
        let symbols = extract(source, "Service.kt");
        assert!(symbols.iter().any(|s| s.name == "PublicService"));
        assert!(symbols.iter().any(|s| s.name == "publicMethod"));
        assert!(
            !symbols.iter().any(|s| s.name == "secretMethod"),
            "private method should be skipped"
        );
    }

    #[test]
    fn test_function_no_return_type() {
        let source = r#"fun doWork(task: String) {
    println(task)
}"#;
        let symbols = extract(source, "Work.kt");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "doWork");
        assert!(symbols[0].return_type.is_none());
    }
}
