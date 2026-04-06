use super::{find_ancestor, find_child_by_kind, node_text, LanguageHooks, Parameter, ReferenceHooks, SymbolInfo, SymbolKind, Visibility};
use tree_sitter::Node;

/// Check modifiers on a declaration node.
/// Returns (has_public, has_protected, has_private, has_static, has_final).
fn check_modifiers(node: &Node, source: &str) -> (bool, bool, bool, bool, bool) {
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

/// Java visibility: only public or protected symbols are visible.
/// Private and package-private (no modifier) are skipped.
/// Also checks ancestor class visibility — members inside a private or package-private
/// class are not externally visible.
fn is_visible(node: &Node, source: &str) -> bool {
    let (public, protected, private, _, _) = check_modifiers(node, source);
    if private || !(public || protected) {
        return false;
    }

    // Check all ancestor classes for visibility — a public method in a package-private
    // or private class (at any nesting level) is not externally visible.
    let mut current = node.parent();
    while let Some(n) = current {
        if matches!(n.kind(), "class_declaration" | "interface_declaration" | "enum_declaration") {
            let (p_public, p_protected, p_private, _, _) = check_modifiers(&n, source);
            if p_private || !(p_public || p_protected) {
                return false;
            }
        }
        current = n.parent();
    }

    true
}

/// For methods and fields inside a class, return the class name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "method_declaration" | "field_declaration" => {
            let class_node = find_ancestor(node, "class_declaration")
                .or_else(|| find_ancestor(node, "interface_declaration"))
                .or_else(|| find_ancestor(node, "enum_declaration"))?;
            let name_node = class_node.child_by_field_name("name")?;
            node_text(&name_node, source).map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Build signature for Java symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum => {
            build_type_signature(node, source)
        }
        SymbolKind::Method | SymbolKind::Function => build_method_signature(node, source),
        SymbolKind::Constant => {
            // Will be overridden in post_process, but provide a default
            let type_str = find_type_node(node, source);
            format!(
                "public static final {} {}",
                type_str.as_deref().unwrap_or("?"),
                name
            )
        }
        _ => format!("{:?} {}", kind, name),
    }
}

/// Build a signature for a type declaration (class, interface, enum).
/// Captures everything from node start up to the body node start.
fn build_type_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let body = find_child_by_kind(node, "class_body")
        .or_else(|| find_child_by_kind(node, "interface_body"))
        .or_else(|| find_child_by_kind(node, "enum_body"));
    let end = body.map(|n| n.start_byte()).unwrap_or(node.end_byte());

    let sig = source[start..end.min(source.len())].trim();
    if !sig.is_empty() {
        sig.to_string()
    } else {
        format!("public {}", node.kind().replace("_declaration", ""))
    }
}

/// Build a signature for a method declaration.
/// Captures everything from node start up to the body block start.
fn build_method_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let end = find_child_by_kind(node, "block")
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
        if child.kind() == "formal_parameter" || child.kind() == "spread_parameter" {
            let name = find_identifier(&child, source).unwrap_or_default();
            let type_ann = find_type_node(&child, source);

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

/// Extract return type from the method node.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    find_type_node(node, source)
}

/// Post-process Java symbols.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    let (public, protected, private, is_static, is_final) = check_modifiers(node, source);

    // Set visibility string
    if public {
        sym.visibility = Visibility::Public;
    } else if protected {
        sym.visibility = Visibility::Protected;
    }

    match sym.kind {
        SymbolKind::Method => {
            // Static methods become Function kind
            if is_static {
                sym.kind = SymbolKind::Function;
            }
            Some(sym)
        }
        SymbolKind::Constant => {
            // Fields: only include if public/protected AND static AND final
            if private || !(public || protected) || !is_static || !is_final {
                return None;
            }

            // Extract name from variable_declarator child
            let declarator = find_child_by_kind(node, "variable_declarator")?;
            let name = find_identifier(&declarator, source)?;

            let type_str = find_type_node(node, source);
            let vis = &sym.visibility;
            let signature = format!(
                "{} static final {} {}",
                vis,
                type_str.as_deref().unwrap_or("?"),
                name
            );

            sym.name = name;
            sym.kind = SymbolKind::Constant;
            sym.signature = Some(signature);
            sym.return_type = None;
            sym.parameters = None;

            Some(sym)
        }
        _ => Some(sym),
    }
}

/// Find the identifier child of a node.
fn find_identifier(node: &Node, source: &str) -> Option<String> {
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

/// Find a type child of a node (type_identifier, integral_type, etc.).
fn find_type_node(node: &Node, source: &str) -> Option<String> {
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

/// Return Java language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        reference_hooks: Some(ReferenceHooks {
            enclosing_ancestors: &[
                "method_declaration",
                "constructor_declaration",
                "class_declaration",
                "interface_declaration",
                "enum_declaration",
            ],
            // Keep only literals/keywords and primitive type names — things that
            // cannot be user-defined. JDK class names like `List`, `Map`,
            // `Optional`, `String`, `Exception` are ORDINARY identifiers in Java
            // and stoplisting them turns any repo-defined type with one of those
            // names into a permanent false negative for reference lookup. Push
            // JDK-noise handling to query/ranking time instead.
            reference_stoplist: &[
                "true", "false", "null", "this", "super",
                "void", "int", "long", "boolean", "double", "float", "byte", "char", "short",
            ],
        }),
    }
}
