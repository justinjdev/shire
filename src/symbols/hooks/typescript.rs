use super::{find_ancestor, node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Visibility filter for TypeScript/JavaScript symbols.
///
/// The query already filters to export_statement, so most symbols are visible.
/// For methods inside classes, skip `#`-prefixed names and nodes with
/// private/protected accessibility modifiers.
fn is_visible(node: &Node, source: &str) -> bool {
    if node.kind() == "method_definition" {
        // Check for #private names
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                if name.starts_with('#') {
                    return false;
                }
            }
        }

        // Check for private/protected accessibility modifier
        for i in 0..node.child_count() {
            let child = node.child(i).unwrap();
            if child.kind() == "accessibility_modifier" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    if text == "private" || text == "protected" {
                        return false;
                    }
                }
            }
        }
    }

    true
}

/// Resolve parent symbol name for methods inside classes.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "method_definition" {
        return None;
    }

    let class_node = find_ancestor(node, "class_declaration")?;
    let name_node = class_node.child_by_field_name("name")?;
    name_node.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())
}

/// Build signature string for TypeScript/JavaScript symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => build_function_signature(node, source),
        SymbolKind::Method => build_method_signature(node, source, name),
        SymbolKind::Class => format!("class {}", name),
        SymbolKind::Interface => format!("interface {}", name),
        SymbolKind::Enum => format!("enum {}", name),
        SymbolKind::Type => {
            // For type aliases, return first line of the node text
            node_text(node, source)
                .map(|t| t.lines().next().unwrap_or(t).to_string())
                .unwrap_or_else(|| format!("type {}", name))
        }
        SymbolKind::Constant => {
            // For constants, get the lexical_declaration parent text (first line)
            if let Some(parent) = node.parent() {
                if parent.kind() == "lexical_declaration" {
                    return node_text(&parent, source)
                        .map(|t| t.lines().next().unwrap_or(t).to_string())
                        .unwrap_or_else(|| format!("const {}", name));
                }
            }
            node_text(node, source)
                .map(|t| t.lines().next().unwrap_or(t).to_string())
                .unwrap_or_else(|| format!("const {}", name))
        }
        _ => format!("{}", name),
    }
}

/// Build function signature: source span from start to end of return_type or parameters.
fn build_function_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let end = node
        .child_by_field_name("return_type")
        .map(|n| n.end_byte())
        .or_else(|| node.child_by_field_name("parameters").map(|n| n.end_byte()))
        .unwrap_or(node.end_byte());

    let text = &source[start..end.min(source.len())];
    text.lines().collect::<Vec<_>>().join(" ").trim().to_string()
}

/// Build method signature: `name(params): return_type`.
fn build_method_signature(node: &Node, source: &str, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .and_then(|n| node_text(&n, source))
        .unwrap_or("()");
    let ret = node
        .child_by_field_name("return_type")
        .and_then(|n| unwrap_type_annotation(&n, source))
        .map(|r| format!(": {}", r))
        .unwrap_or_default();
    format!("{}{}{}", name, params, ret)
}

/// Unwrap a type from a type_annotation node, skipping the colon.
fn unwrap_type_annotation(node: &Node, source: &str) -> Option<String> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() != ":" {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    node.utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.trim_start_matches(": ").to_string())
}

/// Extract parameters from TypeScript/JavaScript function/method nodes.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
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
                    .and_then(|n| unwrap_type_annotation(&n, source));

                if !name.is_empty() {
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            "identifier" => {
                // JS-style simple parameters
                if let Ok(name) = child.utf8_text(source.as_bytes()) {
                    params.push(Parameter {
                        name: name.to_string(),
                        type_annotation: None,
                    });
                }
            }
            _ => {}
        }
    }

    params
}

/// Extract return type hook.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| unwrap_type_annotation(&n, source))
}

/// Post-process symbols.
///
/// For `variable_declarator` nodes (Constant kind): extract name from the
/// variable_declarator's name field, and set signature from the parent
/// lexical_declaration's first line.
///
/// For `type_alias_declaration` nodes: set signature to first line of node text.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    if sym.kind == SymbolKind::Constant && node.kind() == "variable_declarator" {
        // Name is already captured by @name on the variable_declarator's name field.
        // Set signature from the parent lexical_declaration.
        if let Some(parent) = node.parent() {
            if parent.kind() == "lexical_declaration" {
                sym.signature = node_text(&parent, source)
                    .map(|t| t.lines().next().unwrap_or(t).to_string());
            }
        }
    }

    if sym.kind == SymbolKind::Type && node.kind() == "type_alias_declaration" {
        sym.signature = node_text(node, source)
            .map(|t| t.lines().next().unwrap_or(t).to_string());
    }

    Some(sym)
}

/// Return the language hooks for TypeScript and JavaScript.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        enclosing_ancestors: &[],
        reference_stoplist: &[],
    }
}
