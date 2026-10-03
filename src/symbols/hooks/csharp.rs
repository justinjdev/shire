use super::{
    LanguageHooks, Parameter, SymbolInfo, SymbolKind, Visibility, find_ancestor,
    find_child_by_kind, narrow_by_enclosing_types, node_text,
};
use tree_sitter::Node;

/// Check modifiers on a declaration node.
/// C# modifiers are direct children of the declaration node (not wrapped in a `modifiers` node).
/// Returns (has_public, has_protected, has_private, has_internal, has_static, has_readonly).
fn check_modifiers(node: &Node, source: &str) -> (bool, bool, bool, bool, bool, bool) {
    let mut public = false;
    let mut protected = false;
    let mut private = false;
    let mut internal = false;
    let mut is_static = false;
    let mut is_readonly = false;

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind() == "modifier"
        {
            let text = child.utf8_text(source.as_bytes()).unwrap_or("");
            match text {
                "public" => public = true,
                "protected" => protected = true,
                "private" => private = true,
                "internal" => internal = true,
                "static" => is_static = true,
                "readonly" => is_readonly = true,
                _ => {}
            }
        }
    }

    (public, protected, private, internal, is_static, is_readonly)
}

/// Type kinds whose body makes a declaration a member.
const TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "struct_declaration",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
];

/// A declaration's own C# accessibility. `protected internal` and `private
/// protected` both count as protected. With no modifier, an interface member
/// is public, any other member is private, and a top-level type is internal.
fn own_visibility(node: &Node, source: &str) -> Visibility {
    let (public, protected, private, internal, _, _) = check_modifiers(node, source);
    if public {
        return Visibility::Public;
    }
    if protected {
        return Visibility::Protected;
    }
    if private {
        return Visibility::Private;
    }
    if internal {
        return Visibility::Internal;
    }
    match find_enclosing_type(node).map(|t| t.kind()) {
        Some("interface_declaration") => Visibility::Public,
        Some(_) => Visibility::Private,
        None => Visibility::Internal,
    }
}

/// The nearest enclosing type declaration, if `node` is a member of one.
fn find_enclosing_type<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(n) = current {
        if TYPE_KINDS.contains(&n.kind()) {
            return Some(n);
        }
        current = n.parent();
    }
    None
}

/// C# visibility: the declaration's own accessibility, narrowed by every
/// enclosing type — a public method of a private or internal type is not
/// reachable from outside it.
fn visibility(node: &Node, source: &str) -> Visibility {
    narrow_by_enclosing_types(
        node,
        source,
        own_visibility(node, source),
        TYPE_KINDS,
        own_visibility,
    )
}

/// For methods and fields inside a type, return the type name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "method_declaration" | "field_declaration" => {
            let type_node = find_ancestor(node, "class_declaration")
                .or_else(|| find_ancestor(node, "struct_declaration"))
                .or_else(|| find_ancestor(node, "interface_declaration"))
                .or_else(|| find_ancestor(node, "enum_declaration"))
                .or_else(|| find_ancestor(node, "record_declaration"))?;
            let name_node = type_node.child_by_field_name("name")?;
            node_text(&name_node, source).map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Build signature for C# symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum | SymbolKind::Struct => {
            build_type_signature(node, source)
        }
        SymbolKind::Method | SymbolKind::Function => build_method_signature(node, source),
        SymbolKind::Constant => {
            let type_str = find_type_node(node, source);
            format!(
                "public static {} {}",
                type_str.as_deref().unwrap_or("?"),
                name
            )
        }
        _ => format!("{:?} {}", kind, name),
    }
}

/// Build a signature for a type declaration (class, struct, interface, enum, record).
/// Captures everything from node start up to the body node start.
fn build_type_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let body = find_child_by_kind(node, "declaration_list")
        .or_else(|| find_child_by_kind(node, "enum_member_declaration_list"));
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

/// Extract parameters from parameter_list.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "parameter_list") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter" {
            let name = child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .unwrap_or_default()
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
    }

    params
}

/// Extract return type from the method node.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    // C# methods have a `returns` field for the return type
    if let Some(ret) = node.child_by_field_name("returns") {
        return ret.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
    }
    // Fall back to looking for type child nodes
    find_type_node(node, source)
}

/// Post-process C# symbols.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    let (_, _, _, _, is_static, is_readonly) = check_modifiers(node, source);

    match sym.kind {
        SymbolKind::Method => {
            // Static methods become Function kind
            if is_static {
                sym.kind = SymbolKind::Function;
            }
            Some(sym)
        }
        SymbolKind::Constant => {
            // Fields: only `const` or `static readonly` ones are constants.
            // Their accessibility is recorded in `sym.visibility`, not
            // filtered on.
            let is_const = has_modifier(node, source, "const");
            if !(is_const || is_static && is_readonly) {
                return None;
            }

            // Extract name from variable_declarator child
            let var_decl = find_child_by_kind(node, "variable_declaration")?;
            let declarator = find_child_by_kind(&var_decl, "variable_declarator")?;
            let name = find_identifier(&declarator, source)?;

            let type_str = find_type_node(&var_decl, source);
            // The field's own accessibility, not the one narrowed by its
            // enclosing types: the signature reads like the declaration.
            let vis = own_visibility(node, source);
            let signature = if is_const {
                format!(
                    "{} const {} {}",
                    vis,
                    type_str.as_deref().unwrap_or("?"),
                    name
                )
            } else {
                format!(
                    "{} static readonly {} {}",
                    vis,
                    type_str.as_deref().unwrap_or("?"),
                    name
                )
            };

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

/// Check if a specific modifier is present on the node.
fn has_modifier(node: &Node, source: &str, modifier: &str) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i)
            && child.kind() == "modifier"
        {
            let text = child.utf8_text(source.as_bytes()).unwrap_or("");
            if text == modifier {
                return true;
            }
        }
    }
    false
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

/// Find a type child of a node.
fn find_type_node(node: &Node, source: &str) -> Option<String> {
    let type_kinds = [
        "type_identifier",
        "predefined_type",
        "generic_name",
        "array_type",
        "nullable_type",
        "tuple_type",
        "qualified_name",
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

/// Return C# language hooks.
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
