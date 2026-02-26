use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Parser;

/// Extract exported symbols from TypeScript source code.
pub fn extract(source: &str, file_path: &str, is_tsx: bool) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    let language = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    if parser.set_language(&language.into()).is_err() {
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
            "export_statement" => {
                extract_export_statement(source, file_path, &node, &mut symbols);
            }
            _ => {}
        }
    }

    symbols
}

/// Extract exported symbols from JavaScript source code.
pub fn extract_js(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
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
        if node.kind() == "export_statement" {
            extract_export_statement(source, file_path, &node, &mut symbols);
        }
    }

    symbols
}

fn extract_export_statement(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    // Look at the declaration inside the export
    if let Some(declaration) = node.child_by_field_name("declaration") {
        match declaration.kind() {
            "function_declaration" | "function_signature" => {
                if let Some(sym) = extract_function(source, file_path, &declaration) {
                    symbols.push(sym);
                }
            }
            "class_declaration" => {
                extract_class(source, file_path, &declaration, symbols);
            }
            "interface_declaration" => {
                if let Some(sym) = extract_interface(source, file_path, &declaration) {
                    symbols.push(sym);
                }
            }
            "type_alias_declaration" => {
                if let Some(sym) = extract_type_alias(source, file_path, &declaration) {
                    symbols.push(sym);
                }
            }
            "enum_declaration" => {
                if let Some(sym) = extract_enum(source, file_path, &declaration) {
                    symbols.push(sym);
                }
            }
            "lexical_declaration" => {
                extract_const(source, file_path, &declaration, symbols);
            }
            _ => {}
        }
    } else {
        // Handle `export default class/function` (no declaration field)
        for i in 0..node.child_count() {
            let child = node.child(i).unwrap();
            match child.kind() {
                "function_declaration" | "function" => {
                    if let Some(sym) = extract_function(source, file_path, &child) {
                        symbols.push(sym);
                    }
                }
                "class_declaration" | "class" => {
                    extract_class(source, file_path, &child, symbols);
                }
                _ => {}
            }
        }
    }
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
    let signature = build_function_signature(source, node);

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

    // Extract public methods
    if let Some(body) = node.child_by_field_name("body") {
        for i in 0..body.child_count() {
            let child = body.child(i).unwrap();
            if child.kind() == "method_definition" || child.kind() == "public_field_definition" {
                if let Some(method_name) = child.child_by_field_name("name") {
                    let mname = match method_name.utf8_text(source.as_bytes()) {
                        Ok(s) => s.to_string(),
                        Err(_) => continue,
                    };

                    // Skip private methods
                    if mname.starts_with('#') {
                        continue;
                    }

                    // Check for private/protected access modifiers
                    let is_private = has_accessibility_modifier(&child, source, "private")
                        || has_accessibility_modifier(&child, source, "protected");
                    if is_private {
                        continue;
                    }

                    let mline = child.start_position().row + 1;
                    let params = extract_parameters(source, &child);
                    let return_type = extract_return_type(source, &child);
                    let sig = build_method_signature(source, &child, &mname);

                    symbols.push(SymbolInfo {
                        name: mname,
                        kind: SymbolKind::Method,
                        signature: Some(sig),
                        file_path: file_path.to_string(),
                        line: mline,
                        visibility: "public".to_string(),
                        parent_symbol: Some(class_name.clone()),
                        return_type,
                        parameters: Some(params),
                    });
                }
            }
        }
    }
}

fn has_accessibility_modifier(node: &tree_sitter::Node, source: &str, modifier: &str) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "accessibility_modifier" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                if text == modifier {
                    return true;
                }
            }
        }
    }
    false
}

fn extract_interface(
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
    let signature = format!("interface {}", name);

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Interface,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    })
}

fn extract_type_alias(
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
    let text = node.utf8_text(source.as_bytes()).ok()?.to_string();
    // Trim body to keep just the type declaration line
    let signature = text.lines().next().unwrap_or(&text).to_string();

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Type,
        signature: Some(signature),
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
    let signature = format!("enum {}", name);

    Some(SymbolInfo {
        name,
        kind: SymbolKind::Enum,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    })
}

fn extract_const(
    source: &str,
    file_path: &str,
    node: &tree_sitter::Node,
    symbols: &mut Vec<SymbolInfo>,
) {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    let line = child.start_position().row + 1;
                    let text = node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                    let signature = text.lines().next().unwrap_or(&text).to_string();

                    symbols.push(SymbolInfo {
                        name: name.to_string(),
                        kind: SymbolKind::Constant,
                        signature: Some(signature),
                        file_path: file_path.to_string(),
                        line,
                        visibility: "public".to_string(),
                        parent_symbol: None,
                        return_type: None,
                        parameters: None,
                    });
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
            "required_parameter" | "optional_parameter" => {
                let name = child
                    .child_by_field_name("pattern")
                    .or_else(|| child.child_by_field_name("name"))
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("")
                    .to_string();

                let type_ann = child
                    .child_by_field_name("type")
                    .and_then(|n| {
                        // type_annotation node includes the `:` — get the inner type
                        for j in 0..n.child_count() {
                            let tc = n.child(j).unwrap();
                            if tc.kind() != ":" {
                                return tc.utf8_text(source.as_bytes()).ok();
                            }
                        }
                        n.utf8_text(source.as_bytes()).ok()
                    })
                    .map(|s| s.trim_start_matches(": ").to_string());

                if !name.is_empty() {
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            "formal_parameters" => {
                // JS formal parameters
            }
            _ => {
                // For JS-style simple parameters (identifiers)
                if child.kind() == "identifier" {
                    if let Ok(name) = child.utf8_text(source.as_bytes()) {
                        params.push(Parameter {
                            name: name.to_string(),
                            type_annotation: None,
                        });
                    }
                }
            }
        }
    }

    params
}

fn extract_return_type(source: &str, node: &tree_sitter::Node) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| {
            // The return_type field includes the `:`, we want just the type
            // Find the actual type node inside
            if n.child_count() > 0 {
                // Skip the colon, get the type annotation
                for i in 0..n.child_count() {
                    let child = n.child(i).unwrap();
                    if child.kind() != ":" {
                        return child.utf8_text(source.as_bytes()).ok();
                    }
                }
            }
            n.utf8_text(source.as_bytes()).ok()
        })
        .map(|s| s.trim_start_matches(": ").to_string())
}

fn build_function_signature(source: &str, node: &tree_sitter::Node) -> String {
    // Get text from start of function to end of return type or end of params
    let start = node.start_byte();
    let end = node
        .child_by_field_name("return_type")
        .map(|n| n.end_byte())
        .or_else(|| node.child_by_field_name("parameters").map(|n| n.end_byte()))
        .unwrap_or(node.end_byte());

    let text = &source[start..end.min(source.len())];
    // Take first logical line
    text.lines()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn build_method_signature(source: &str, node: &tree_sitter::Node, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .unwrap_or("()");
    let ret = extract_return_type(source, node)
        .map(|r| format!(": {}", r))
        .unwrap_or_default();
    format!("{}{}{}", name, params, ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_exported_function() {
        let source = r#"export function processPayment(amount: number, currency: string): Promise<Receipt> {
    return fetch('/pay');
}"#;
        let symbols = extract(source, "src/pay.ts", false);
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "processPayment");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.signature.as_ref().unwrap().contains("processPayment"));
        assert_eq!(sym.return_type.as_deref(), Some("Promise<Receipt>"));
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "amount");
        assert_eq!(params[0].type_annotation.as_deref(), Some("number"));
        assert_eq!(params[1].name, "currency");
        assert_eq!(params[1].type_annotation.as_deref(), Some("string"));
    }

    #[test]
    fn test_extract_exported_class_with_methods() {
        let source = r#"export class AuthService {
    validate(token: string): boolean {
        return true;
    }
    private _internal(): void {}
}"#;
        let symbols = extract(source, "src/auth.ts", false);
        assert!(symbols.len() >= 2);
        assert_eq!(symbols[0].name, "AuthService");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        assert_eq!(symbols[1].name, "validate");
        assert_eq!(symbols[1].kind, SymbolKind::Method);
        assert_eq!(symbols[1].parent_symbol.as_deref(), Some("AuthService"));
        // _internal should be skipped (private)
        assert!(!symbols.iter().any(|s| s.name == "_internal"));
    }

    #[test]
    fn test_extract_exported_interface() {
        let source = r#"export interface UserConfig {
    name: string;
    theme: string;
}"#;
        let symbols = extract(source, "src/types.ts", false);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "UserConfig");
        assert_eq!(symbols[0].kind, SymbolKind::Interface);
    }

    #[test]
    fn test_extract_exported_type_alias() {
        let source = "export type Result<T> = Success<T> | Failure;";
        let symbols = extract(source, "src/types.ts", false);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Result");
        assert_eq!(symbols[0].kind, SymbolKind::Type);
    }

    #[test]
    fn test_extract_exported_enum() {
        let source = r#"export enum Status {
    Active,
    Inactive
}"#;
        let symbols = extract(source, "src/types.ts", false);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Status");
        assert_eq!(symbols[0].kind, SymbolKind::Enum);
    }

    #[test]
    fn test_extract_exported_const() {
        let source = "export const MAX_RETRIES = 3;";
        let symbols = extract(source, "src/config.ts", false);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "MAX_RETRIES");
        assert_eq!(symbols[0].kind, SymbolKind::Constant);
    }

    #[test]
    fn test_skip_non_exported() {
        let source = r#"
function internalHelper() {}
class InternalClass {}
const secret = 42;
"#;
        let symbols = extract(source, "src/internal.ts", false);
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_extract_default_export_function() {
        let source = r#"export default function handler(req: Request): Response {
    return new Response();
}"#;
        let symbols = extract(source, "src/handler.ts", false);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "handler");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_js_function() {
        let source = r#"export function greet(name) {
    return 'Hello ' + name;
}"#;
        let symbols = extract_js(source, "src/greet.js");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
    }
}
