use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Parser;

/// Extract public and protected symbols from Java source code.
/// Skips private and package-private (no modifier) declarations.
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
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
                extract_class(source, file_path, &node, &mut symbols);
            }
            "interface_declaration" => {
                extract_interface(source, file_path, &node, &mut symbols);
            }
            "enum_declaration" => {
                extract_enum(source, file_path, &node, &mut symbols);
            }
            _ => {}
        }
    }

    symbols
}

/// Check modifiers on a declaration node and return (has_public, has_protected, has_private, has_static, has_final).
fn check_modifiers(source: &str, node: &tree_sitter::Node) -> (bool, bool, bool, bool, bool) {
    let mut public = false;
    let mut protected = false;
    let mut private = false;
    let mut is_static = false;
    let mut is_final = false;

    if let Some(mods) = find_child_by_kind(node, "modifiers") {
        for i in 0..mods.child_count() {
            if let Some(child) = mods.child(i) {
                let text = child.utf8_text(source.as_bytes()).unwrap_or("");
                match text {
                    "public" => public = true,
                    "protected" => protected = true,
                    "private" => private = true,
                    "static" => is_static = true,
                    "final" => is_final = true,
                    _ => {}
                }
            }
        }
    }

    (public, protected, private, is_static, is_final)
}

/// Returns true if the declaration has public or protected visibility.
fn is_visible(source: &str, node: &tree_sitter::Node) -> bool {
    let (public, protected, private, _, _) = check_modifiers(source, node);
    !private && (public || protected)
}

/// Return "public" or "protected" based on modifiers.
fn visibility_str(source: &str, node: &tree_sitter::Node) -> String {
    let (public, _protected, _, _, _) = check_modifiers(source, node);
    if public {
        "public".to_string()
    } else {
        "protected".to_string()
    }
}

fn extract_class(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    if !is_visible(source, node) {
        return;
    }

    let name = match find_identifier(source, node) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row + 1;
    let signature = build_type_signature(source, node, "class");

    symbols.push(SymbolInfo {
        name: name.clone(),
        kind: SymbolKind::Class,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: visibility_str(source, node),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });

    // Extract members from the class body
    if let Some(body) = node.child_by_field_name("body") {
        extract_class_members(source, file_path, &body, &name, symbols);
    }
}

fn extract_interface(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    if !is_visible(source, node) {
        return;
    }

    let name = match find_identifier(source, node) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row + 1;
    let signature = build_type_signature(source, node, "interface");

    symbols.push(SymbolInfo {
        name,
        kind: SymbolKind::Interface,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: visibility_str(source, node),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });
}

fn extract_enum(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    if !is_visible(source, node) {
        return;
    }

    let name = match find_identifier(source, node) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row + 1;
    let signature = build_type_signature(source, node, "enum");

    symbols.push(SymbolInfo {
        name,
        kind: SymbolKind::Enum,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: visibility_str(source, node),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });
}

fn extract_class_members(
    source: &str,
    file_path: &str,
    body: &tree_sitter::Node,
    class_name: &str,
    symbols: &mut Vec<SymbolInfo>,
) {
    for i in 0..body.child_count() {
        let child = body.child(i).unwrap();
        match child.kind() {
            "method_declaration" => {
                if let Some(sym) = extract_method(source, file_path, &child, class_name) {
                    symbols.push(sym);
                }
            }
            "field_declaration" => {
                if let Some(sym) = extract_constant(source, file_path, &child, class_name) {
                    symbols.push(sym);
                }
            }
            _ => {}
        }
    }
}

fn extract_method(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    class_name: &str,
) -> Option<SymbolInfo> {
    if !is_visible(source, node) {
        return None;
    }

    let name = find_identifier(source, node)?;

    let (_, _, _, is_static, _) = check_modifiers(source, node);

    let line = node.start_position().row + 1;
    let params = extract_parameters(source, node);
    let return_type = find_type_node(source, node);
    let signature = build_method_signature(source, node);

    let kind = if is_static {
        SymbolKind::Function
    } else {
        SymbolKind::Method
    };

    Some(SymbolInfo {
        name,
        kind,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: visibility_str(source, node),
        parent_symbol: Some(class_name.to_string()),
        return_type,
        parameters: Some(params),
    })
}

fn extract_constant(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    class_name: &str,
) -> Option<SymbolInfo> {
    let (public, protected, private, is_static, is_final) = check_modifiers(source, node);

    // Must be public or protected, static, and final
    if private || !(public || protected) || !is_static || !is_final {
        return None;
    }

    // Find the variable declarator to get the name
    let declarator = find_child_by_kind(node, "variable_declarator")?;
    let name = find_identifier(source, &declarator)?;

    let line = node.start_position().row + 1;

    let type_str = find_type_node(source, node);

    let signature = format!(
        "public static final {} {}",
        type_str.as_deref().unwrap_or("?"),
        name
    );

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Constant,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: visibility_str(source, node),
        parent_symbol: Some(class_name.to_string()),
        return_type: None,
        parameters: None,
    })
}

fn extract_parameters(source: &str, node: &tree_sitter::Node) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "formal_parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "formal_parameter" || child.kind() == "spread_parameter" {
            let name = find_identifier(source, &child).unwrap_or_default();
            let type_ann = find_type_node(source, &child);

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

/// Find the identifier child of a node (class name, method name, variable name).
fn find_identifier(source: &str, node: &tree_sitter::Node) -> Option<String> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "identifier" {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Find the type child of a node. In tree-sitter-java, types can be various kinds
/// like type_identifier, integral_type, generic_type, array_type, etc.
fn find_type_node(source: &str, node: &tree_sitter::Node) -> Option<String> {
    let type_kinds = [
        "type_identifier",
        "integral_type",
        "floating_point_type",
        "boolean_type",
        "void_type",
        "generic_type",
        "array_type",
        "scoped_type_identifier",
    ];
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if type_kinds.contains(&child.kind()) {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Build a signature for a type declaration (class, interface, enum).
/// Captures everything up to the opening brace.
fn build_type_signature(source: &str, node: &tree_sitter::Node, keyword: &str) -> String {
    let start = node.start_byte();
    let body = find_child_by_kind(node, "class_body")
        .or_else(|| find_child_by_kind(node, "interface_body"))
        .or_else(|| find_child_by_kind(node, "enum_body"));
    let end = body.map(|n| n.start_byte()).unwrap_or(node.end_byte());

    let sig = source[start..end.min(source.len())].trim();

    if !sig.is_empty() {
        sig.to_string()
    } else {
        let name = find_identifier(source, node).unwrap_or_else(|| "?".to_string());
        format!("public {} {}", keyword, name)
    }
}

/// Build a signature for a method declaration.
/// Captures everything up to the body opening brace.
fn build_method_signature(source: &str, node: &tree_sitter::Node) -> String {
    let start = node.start_byte();
    let end = find_child_by_kind(node, "block")
        .map(|n| n.start_byte())
        .unwrap_or(node.end_byte());

    let sig = source[start..end.min(source.len())].trim();
    sig.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_public_class() {
        let source = r#"
public class UserService {
    private int count;
}
"#;
        let symbols = extract(source, "UserService.java");
        let classes: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "UserService");
        assert_eq!(classes[0].visibility, "public");
        assert!(classes[0].signature.as_ref().unwrap().contains("class"));
        assert!(classes[0]
            .signature
            .as_ref()
            .unwrap()
            .contains("UserService"));
    }

    #[test]
    fn test_extract_public_interface() {
        let source = r#"
public interface Repository<T> {
    T findById(long id);
}
"#;
        let symbols = extract(source, "Repository.java");
        let ifaces: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .collect();
        assert_eq!(ifaces.len(), 1);
        assert_eq!(ifaces[0].name, "Repository");
        assert_eq!(ifaces[0].kind, SymbolKind::Interface);
        assert_eq!(ifaces[0].visibility, "public");
    }

    #[test]
    fn test_extract_public_enum() {
        let source = r#"
public enum Status {
    ACTIVE,
    INACTIVE,
    PENDING
}
"#;
        let symbols = extract(source, "Status.java");
        let enums: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Status");
        assert_eq!(enums[0].kind, SymbolKind::Enum);
        assert_eq!(enums[0].visibility, "public");
    }

    #[test]
    fn test_extract_public_method_with_params_and_return() {
        let source = r#"
public class OrderService {
    public Order processOrder(String customerId, int quantity) {
        return null;
    }
}
"#;
        let symbols = extract(source, "OrderService.java");
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        let m = &methods[0];
        assert_eq!(m.name, "processOrder");
        assert_eq!(m.kind, SymbolKind::Method);
        assert_eq!(m.visibility, "public");
        assert_eq!(m.parent_symbol.as_deref(), Some("OrderService"));
        assert_eq!(m.return_type.as_deref(), Some("Order"));

        let params = m.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "customerId");
        assert_eq!(params[0].type_annotation.as_deref(), Some("String"));
        assert_eq!(params[1].name, "quantity");
        assert_eq!(params[1].type_annotation.as_deref(), Some("int"));
    }

    #[test]
    fn test_extract_protected_method() {
        let source = r#"
public class BaseController {
    protected void handleError(Exception ex) {
    }
}
"#;
        let symbols = extract(source, "BaseController.java");
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "handleError");
        assert_eq!(methods[0].visibility, "protected");
        assert_eq!(
            methods[0].parent_symbol.as_deref(),
            Some("BaseController")
        );
    }

    #[test]
    fn test_extract_static_method_as_function() {
        let source = r#"
public class MathUtils {
    public static double calculateArea(double radius) {
        return Math.PI * radius * radius;
    }
}
"#;
        let symbols = extract(source, "MathUtils.java");
        let funcs: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "calculateArea");
        assert_eq!(funcs[0].kind, SymbolKind::Function);
        assert_eq!(funcs[0].parent_symbol.as_deref(), Some("MathUtils"));
        assert_eq!(funcs[0].return_type.as_deref(), Some("double"));

        let params = funcs[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "radius");
        assert_eq!(params[0].type_annotation.as_deref(), Some("double"));
    }

    #[test]
    fn test_extract_constant() {
        let source = r#"
public class AppConfig {
    public static final String API_VERSION = "v2";
    public static final int MAX_RETRIES = 3;
    private static final String SECRET = "hidden";
}
"#;
        let symbols = extract(source, "AppConfig.java");
        let constants: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        assert_eq!(constants.len(), 2);
        assert_eq!(constants[0].name, "API_VERSION");
        assert_eq!(
            constants[0].signature.as_deref(),
            Some("public static final String API_VERSION")
        );
        assert_eq!(constants[0].parent_symbol.as_deref(), Some("AppConfig"));
        assert_eq!(constants[1].name, "MAX_RETRIES");
    }

    #[test]
    fn test_skip_private_class() {
        let source = r#"
private class InternalHelper {
    public void doSomething() {}
}
"#;
        let symbols = extract(source, "InternalHelper.java");
        // The private class itself should be skipped, and therefore its members too
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_skip_package_private_method() {
        let source = r#"
public class Service {
    void internalMethod(String data) {
    }

    private void secretMethod() {
    }

    public void publicMethod() {
    }
}
"#;
        let symbols = extract(source, "Service.java");
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method || s.kind == SymbolKind::Function)
            .collect();
        // Only publicMethod should be extracted; internalMethod (package-private) and
        // secretMethod (private) should be skipped
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "publicMethod");
    }

    #[test]
    fn test_line_numbers() {
        let source = r#"package com.example;

public class Demo {
    public void first() {}
    public void second() {}
}
"#;
        let symbols = extract(source, "Demo.java");
        let class_sym = symbols.iter().find(|s| s.name == "Demo").unwrap();
        assert_eq!(class_sym.line, 3);

        let first = symbols.iter().find(|s| s.name == "first").unwrap();
        let second = symbols.iter().find(|s| s.name == "second").unwrap();
        assert!(first.line < second.line);
    }
}
