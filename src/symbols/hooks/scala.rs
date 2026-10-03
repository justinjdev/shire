use super::{
    LanguageHooks, Parameter, ReferenceHooks, SymbolInfo, SymbolKind, Visibility, find_ancestor,
    find_child_by_kind, modifier_text, narrow_by_enclosing_types,
};
use tree_sitter::Node;

/// Scala type-defining node kinds.
const TYPE_NODES: &[&str] = &[
    "class_definition",
    "object_definition",
    "trait_definition",
    "enum_definition",
];

/// A declaration's own Scala access modifier; no modifier means public.
/// `private[pkg]` is visible throughout `pkg`, so it is internal;
/// `private` and `private[this]` are private.
fn own_visibility(node: &Node, source: &str) -> Visibility {
    let Some(text) = modifier_text(node, source, "access_modifier") else {
        return Visibility::Public;
    };
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if text.starts_with("protected") {
        Visibility::Protected
    } else if text == "private" || text == "private[this]" {
        Visibility::Private
    } else if text.starts_with("private") {
        Visibility::Internal
    } else {
        Visibility::Public
    }
}

/// Scala visibility: the declaration's own modifier, narrowed by every
/// enclosing class/object/trait/enum.
fn visibility(node: &Node, source: &str) -> Visibility {
    narrow_by_enclosing_types(
        node,
        source,
        own_visibility(node, source),
        TYPE_NODES,
        own_visibility,
    )
}

/// For methods inside class/object/trait/enum bodies, resolve the parent type name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    let kind = node.kind();
    if kind != "function_definition" && kind != "function_declaration" {
        return None;
    }

    // Methods live inside template_body or enum_body; walk up to find the enclosing type.
    for &type_node in TYPE_NODES {
        if let Some(parent) = find_ancestor(node, type_node) {
            return parent
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());
        }
    }

    None
}

/// Build a signature string for a Scala symbol.
///
/// For functions/methods: source span from node start up to (but not including) the body.
/// For types: "class Name", "object Name", "trait Name", "enum Name".
/// For type aliases: "type Name".
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => {
            let start = node.start_byte();
            let end = find_child_by_kind(node, "block")
                .or_else(|| find_child_by_kind(node, "indented_block"))
                .or_else(|| find_child_by_kind(node, "="))
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            source[start..end.min(source.len())].trim().to_string()
        }
        _ => {
            let keyword = detect_type_keyword(node);
            format!("{} {}", keyword, name)
        }
    }
}

/// Extract parameters from a `parameters` node.
///
/// Each `parameter` child has a `name` (identifier) and optionally a colon followed
/// by a type annotation.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "parameters") {
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
fn find_param_name(source: &str, param_node: &Node) -> Option<String> {
    param_node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
        .or_else(|| {
            // Fallback: look for first identifier child
            for i in 0..param_node.child_count() {
                let child = param_node.child(i).unwrap();
                if child.kind() == "identifier" {
                    return child
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                }
            }
            None
        })
}

/// Extract the type annotation from a `parameter` node.
/// A parameter is structured as: name ":" type
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

/// Extract the return type from a function definition/declaration.
/// The return type follows a ":" after the `parameters` node.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Detect the type keyword for a node.
fn detect_type_keyword(node: &Node) -> &'static str {
    match node.kind() {
        "object_definition" => "object",
        "trait_definition" => "trait",
        "enum_definition" => "enum",
        "type_definition" => "type",
        "class_definition" => "class",
        _ => "class",
    }
}

/// Post-process: for `object_definition` nodes, keep as Class kind (signature already
/// shows "object Name"). For `trait_definition`, ensure kind is Interface.
/// For `enum_definition`, ensure kind is Enum.
fn post_process(mut sym: SymbolInfo, node: &Node, _source: &str) -> Option<SymbolInfo> {
    match node.kind() {
        "trait_definition" => {
            sym.kind = SymbolKind::Interface;
        }
        "enum_definition" => {
            sym.kind = SymbolKind::Enum;
        }
        "type_definition" => {
            sym.kind = SymbolKind::Type;
        }
        "object_definition" => {
            // Keep as Class — signature already reads "object Name"
        }
        _ => {}
    }
    Some(sym)
}

/// Return Scala language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_definition: None,
        visibility: Some(visibility),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        reference_hooks: Some(ReferenceHooks {
            enclosing_ancestors: &[
                "function_definition",
                "class_definition",
                "object_definition",
                "trait_definition",
            ],
            reference_stoplist: &[
                "true", "false", "null", "this", "super", "Int", "Long", "Short", "Byte", "Float",
                "Double", "Boolean", "Char", "String", "Unit", "Some", "None", "Option", "List",
                "Seq", "Map", "Set", "Array", "println", "print",
            ],
        }),
    }
}
