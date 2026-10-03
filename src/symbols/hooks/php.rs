use super::{
    LanguageHooks, Parameter, SymbolInfo, SymbolKind, Visibility, find_ancestor,
    find_child_by_kind, node_text,
};
use tree_sitter::Node;

/// Check visibility and other modifiers on a PHP declaration node.
/// PHP uses individual modifier nodes as direct children rather than a modifiers wrapper.
/// Returns (has_public, has_protected, has_private, has_static).
fn check_modifiers(node: &Node, source: &str) -> (bool, bool, bool, bool) {
    let mut public = false;
    let mut protected = false;
    let mut private = false;
    let mut is_static = false;

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "visibility_modifier" => {
                    let text = child.utf8_text(source.as_bytes()).unwrap_or("");
                    match text {
                        "public" => public = true,
                        "protected" => protected = true,
                        "private" => private = true,
                        _ => {}
                    }
                }
                "static_modifier" => is_static = true,
                _ => {}
            }
        }
    }

    (public, protected, private, is_static)
}

/// PHP visibility from the member's `public`/`protected`/`private` modifier;
/// no modifier means public (PHP's default). PHP types themselves carry no
/// access modifier, so nothing is narrowed by its enclosing class.
fn visibility(node: &Node, source: &str) -> Visibility {
    match check_modifiers(node, source) {
        (_, _, true, _) => Visibility::Private,
        (_, true, _, _) => Visibility::Protected,
        _ => Visibility::Public,
    }
}

/// For methods and constants inside a class/interface/trait/enum, return the type name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "method_declaration" | "const_declaration" => {
            let parent_node = find_ancestor(node, "class_declaration")
                .or_else(|| find_ancestor(node, "interface_declaration"))
                .or_else(|| find_ancestor(node, "trait_declaration"))
                .or_else(|| find_ancestor(node, "enum_declaration"))?;
            let name_node = parent_node.child_by_field_name("name")?;
            node_text(&name_node, source).map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Build signature for PHP symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Trait | SymbolKind::Enum => {
            build_type_signature(node, source, kind)
        }
        SymbolKind::Method | SymbolKind::Function => build_callable_signature(node, source),
        SymbolKind::Constant => {
            format!("const {}", name)
        }
        _ => format!("{:?} {}", kind, name),
    }
}

/// Build a signature for a type declaration.
/// Captures everything from node start up to the body node start.
fn build_type_signature(node: &Node, source: &str, kind: SymbolKind) -> String {
    let start = node.start_byte();
    let body = find_child_by_kind(node, "declaration_list")
        .or_else(|| find_child_by_kind(node, "enum_declaration_list"));
    let end = body.map(|n| n.start_byte()).unwrap_or(node.end_byte());

    let sig = source[start..end.min(source.len())].trim();
    if !sig.is_empty() {
        sig.to_string()
    } else {
        let kind_str = match kind {
            SymbolKind::Interface => "interface",
            SymbolKind::Trait => "trait",
            SymbolKind::Enum => "enum",
            _ => "class",
        };
        format!(
            "{} {}",
            kind_str,
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or("?")
        )
    }
}

/// Build a signature for a function or method declaration.
/// Captures everything from node start up to the body block start.
fn build_callable_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let end = find_child_by_kind(node, "compound_statement")
        .map(|n| n.start_byte())
        .unwrap_or(node.end_byte());

    source[start..end.min(source.len())].trim().to_string()
}

/// Extract parameters from formal_parameters.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "formal_parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "simple_parameter" || child.kind() == "variadic_parameter" {
            let name = child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();

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
    }

    params
}

/// Extract return type from the function/method node.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| {
            // Strip the leading `: ` if present
            let trimmed = s.trim();
            if let Some(stripped) = trimmed.strip_prefix(':') {
                stripped.trim().to_string()
            } else {
                trimmed.to_string()
            }
        })
}

/// Post-process PHP symbols.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    let (_, _, _, is_static) = check_modifiers(node, source);

    match sym.kind {
        SymbolKind::Method => {
            // Static methods become Function kind
            if is_static {
                sym.kind = SymbolKind::Function;
            }
            Some(sym)
        }
        _ => Some(sym),
    }
}

/// Return PHP language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_definition: None,
        visibility: Some(visibility),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        reference_hooks: None,
    }
}
