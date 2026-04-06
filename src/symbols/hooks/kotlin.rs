use super::{find_ancestor, find_child_by_kind, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Check whether a Kotlin symbol is visible (not private or internal).
/// Kotlin defaults to public visibility.
/// Also checks ancestor class visibility — methods inside a private class are not visible.
fn is_visible(node: &Node, source: &str) -> bool {
    if has_private_or_internal_modifier(node, source) {
        return false;
    }

    // Check all ancestor classes/objects for visibility
    let mut current = node.parent();
    while let Some(n) = current {
        if (n.kind() == "class_declaration" || n.kind() == "object_declaration")
            && has_private_or_internal_modifier(&n, source)
        {
            return false;
        }
        current = n.parent();
    }

    true
}

/// Check if a node has private or internal visibility modifier.
fn has_private_or_internal_modifier(node: &Node, source: &str) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "modifiers" {
            for j in 0..child.child_count() {
                let modifier = child.child(j).unwrap();
                if modifier.kind() == "visibility_modifier"
                    && let Ok(text) = modifier.utf8_text(source.as_bytes())
                        && (text == "private" || text == "internal") {
                            return true;
                        }
            }
        }
    }
    false
}

/// For methods inside class/object bodies, resolve the parent class or object name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "function_declaration" {
        return None;
    }

    // Methods live inside class_body or enum_class_body; walk up to find the
    // enclosing class_declaration or object_declaration.
    let parent = find_ancestor(node, "class_declaration")
        .or_else(|| find_ancestor(node, "object_declaration"))?;

    parent
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Build a signature string for a Kotlin symbol.
///
/// For functions/methods: source span from node start up to (but not including) the
/// function body.
/// For classes: `"class Name"`, `"interface Name"`, `"enum class Name"`, or
/// `"object Name"` depending on keyword children.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let start = node.start_byte();
            let end = find_child_by_kind(node, "function_body")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        _ => {
            let keyword = detect_class_keyword(node, source);
            format!("{} {}", keyword, name)
        }
    }
}

/// Extract parameters from `function_value_parameters`.
///
/// Each `parameter` child contains an `identifier` (the name) and optionally a
/// `type` node after a `:` separator.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "function_value_parameters") {
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

/// Find the identifier name inside a parameter node.
/// The parameter node has `identifier` children (no `name` field).
fn find_param_name(source: &str, param_node: &Node) -> Option<String> {
    for i in 0..param_node.child_count() {
        let child = param_node.child(i).unwrap();
        match child.kind() {
            "identifier" | "simple_identifier" => {
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

/// Extract the type annotation from a `parameter` node.
/// A parameter is structured as: identifier ":" type
fn extract_parameter_type(source: &str, param_node: &Node) -> Option<String> {
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
/// The return type follows a ":" after the `function_value_parameters`.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
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

/// Detect the class-like keyword from a class_declaration or object_declaration node.
fn detect_class_keyword(node: &Node, source: &str) -> &'static str {
    if node.kind() == "object_declaration" {
        return "object";
    }
    let mut saw_enum = false;
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if let Ok(text) = child.utf8_text(source.as_bytes()) {
            match text {
                "interface" => return "interface",
                "enum" => saw_enum = true,
                "class" if saw_enum => return "enum class",
                "class" => return "class",
                _ => {}
            }
        }
    }
    "class"
}

/// Post-process: for class_declaration nodes, determine the actual kind by scanning
/// keyword children: "interface" -> Interface, "enum" -> Enum, else Class.
/// For object_declaration, keep as Class (signature already set to "object Name").
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    if node.kind() == "class_declaration" && matches!(sym.kind, SymbolKind::Class) {
        sym.kind = match detect_class_keyword(node, source) {
            "interface" => SymbolKind::Interface,
            "enum class" => SymbolKind::Enum,
            _ => SymbolKind::Class,
        };
    }
    Some(sym)
}

/// Return Kotlin language hooks.
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
