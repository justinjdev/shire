use super::{LanguageHooks, Parameter, SymbolInfo, SymbolKind, find_ancestor, find_child_by_kind};
use tree_sitter::Node;

/// Check whether a Swift symbol is visible (not private or fileprivate).
/// Swift defaults to `internal` visibility, which is visible within the module.
/// Also checks ancestor class visibility — methods inside a private class are not visible.
fn is_visible(node: &Node, source: &str) -> bool {
    if has_private_modifier(node, source) {
        return false;
    }

    // Check all ancestor classes/protocols for visibility
    let mut current = node.parent();
    while let Some(n) = current {
        if (n.kind() == "class_declaration" || n.kind() == "protocol_declaration")
            && has_private_modifier(&n, source)
        {
            return false;
        }
        current = n.parent();
    }

    true
}

/// Check if a node has private or fileprivate visibility modifier.
/// In this grammar, visibility_modifier may be inside a `modifiers` wrapper node.
fn has_private_modifier(node: &Node, source: &str) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "visibility_modifier"
            && let Ok(text) = child.utf8_text(source.as_bytes())
            && (text == "private" || text == "fileprivate")
        {
            return true;
        }
        if child.kind() == "modifiers" {
            for j in 0..child.child_count() {
                let grandchild = child.child(j).unwrap();
                if grandchild.kind() == "visibility_modifier"
                    && let Ok(text) = grandchild.utf8_text(source.as_bytes())
                    && (text == "private" || text == "fileprivate")
                {
                    return true;
                }
            }
        }
    }
    false
}

/// For methods inside class/struct/enum/actor or protocol bodies, resolve the parent type name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "function_declaration" && node.kind() != "protocol_function_declaration" {
        return None;
    }

    // Methods live inside class_body or protocol_body; walk up to find the
    // enclosing class_declaration or protocol_declaration.
    let parent = find_ancestor(node, "class_declaration")
        .or_else(|| find_ancestor(node, "protocol_declaration"))?;

    parent
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Build a signature string for a Swift symbol.
///
/// For functions/methods: source span from node start up to (but not including) the
/// function body.
/// For types: `"class Name"`, `"struct Name"`, `"enum Name"`, `"protocol Name"`,
/// or `"actor Name"` depending on keyword children.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let start = node.start_byte();
            let end = find_child_by_kind(node, "function_body")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        SymbolKind::Interface => {
            format!("protocol {}", name)
        }
        _ => {
            let keyword = detect_class_keyword(node, source);
            format!("{} {}", keyword, name)
        }
    }
}

/// Extract parameters from a Swift function declaration.
///
/// Swift parameters can have external and internal names. The grammar uses
/// `parameter` children inside a parameter list node.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    // Find the parameter list (may be named differently in the grammar)
    let params_node = match find_parameter_list(node) {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter" {
            let name = find_param_name(source, &child).unwrap_or_default();
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

/// Find the parameter list child node. Swift grammar may use different node names.
fn find_parameter_list<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        let kind = child.kind();
        // tree-sitter-swift uses nodes like "lambda_function_type_parameters" or
        // "function_type_parameters" — match any *_parameter* container except
        // individual "parameter" nodes
        if kind.contains("parameter") && kind != "parameter" {
            return Some(child);
        }
    }
    // Fallback: if no dedicated parameter list node, look for `(` ... `)` containing parameters
    None
}

/// Find the parameter name inside a parameter node.
/// Swift parameters may have external_name and internal_name (both simple_identifier).
fn find_param_name(source: &str, param_node: &Node) -> Option<String> {
    // Look for the internal name (second simple_identifier) or fall back to first
    let mut identifiers = Vec::new();
    for i in 0..param_node.child_count() {
        let child = param_node.child(i).unwrap();
        if child.kind() == "simple_identifier"
            && let Ok(text) = child.utf8_text(source.as_bytes())
        {
            identifiers.push(text.to_string());
        }
    }

    match identifiers.len() {
        0 => None,
        1 => Some(identifiers.into_iter().next().unwrap()),
        // With two identifiers, the first is the external name and second is internal
        _ => Some(identifiers.into_iter().last().unwrap()),
    }
}

/// Extract the type annotation from a `parameter` node.
/// Swift parameter types follow the `:` separator.
fn extract_parameter_type(source: &str, param_node: &Node) -> Option<String> {
    let mut found_colon = false;
    for i in 0..param_node.child_count() {
        let child = param_node.child(i).unwrap();
        if child.kind() == ":" {
            found_colon = true;
            continue;
        }
        if found_colon {
            // Skip simple_identifier nodes (those are parameter names before the colon
            // in some grammar versions), and return the first type-like node
            let kind = child.kind();
            if kind == "simple_identifier" || kind == "," {
                continue;
            }
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    None
}

/// Extract the return type from a function_declaration.
/// Swift return types follow the `->` arrow operator.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    let mut found_arrow = false;

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();

        if child.kind() == "arrow_operator" || child.utf8_text(source.as_bytes()).ok() == Some("->")
        {
            found_arrow = true;
            continue;
        }

        if found_arrow {
            // Skip the function body
            if child.kind() == "function_body" {
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

/// Detect the class-like keyword from a class_declaration node.
/// Swift uses a single `class_declaration` for class, struct, enum, extension, and actor.
fn detect_class_keyword(node: &Node, source: &str) -> &'static str {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if let Ok(text) = child.utf8_text(source.as_bytes()) {
            match text {
                "struct" => return "struct",
                "enum" => return "enum",
                "actor" => return "actor",
                "extension" => return "extension",
                "class" => return "class",
                _ => {}
            }
        }
    }
    "class"
}

/// Post-process: for class_declaration nodes, determine the actual kind by scanning
/// keyword children: "struct" -> Struct, "enum" -> Enum, "actor" -> Class, "class" -> Class.
/// Skip "extension" declarations (they extend existing types, not define new ones).
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    if node.kind() == "class_declaration" && matches!(sym.kind, SymbolKind::Class) {
        let keyword = detect_class_keyword(node, source);
        match keyword {
            "struct" => sym.kind = SymbolKind::Struct,
            "enum" => sym.kind = SymbolKind::Enum,
            "extension" => return None,        // Skip extensions
            _ => sym.kind = SymbolKind::Class, // class and actor
        }
    }
    Some(sym)
}

/// Return Swift language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        reference_hooks: None,
    }
}
