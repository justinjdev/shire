use super::{
    LanguageHooks, Parameter, SymbolInfo, SymbolKind, find_ancestor, find_child_by_kind, node_text,
};
use tree_sitter::Node;

/// C++ visibility: include all symbols.
/// C++ exposes APIs through headers and visibility semantics are complex
/// (friend, nested classes, etc.), so we don't filter by access specifiers.
fn is_visible(_node: &Node, _source: &str) -> bool {
    true
}

/// Resolve parent: if inside a class_specifier or struct_specifier, return the class/struct name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    let parent_node = find_ancestor(node, "class_specifier")
        .or_else(|| find_ancestor(node, "struct_specifier"))?;
    let name_node = parent_node.child_by_field_name("name")?;
    node_text(&name_node, source).map(|s| s.to_string())
}

/// Build signature for C++ symbols.
/// For functions/methods: source from node start to the body opening brace.
/// For classes/structs/enums/namespaces: "kind Name".
/// For type aliases: the full alias text.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Class => {
            if node.kind() == "namespace_definition" {
                format!("namespace {}", name)
            } else {
                format!("class {}", name)
            }
        }
        SymbolKind::Struct => format!("struct {}", name),
        SymbolKind::Enum => format!("enum {}", name),
        SymbolKind::Type => {
            // For alias declarations, extract the full text
            node_text(node, source)
                .map(|s| s.trim().trim_end_matches(';').trim().to_string())
                .unwrap_or_else(|| format!("using {}", name))
        }
        _ => {
            // function_definition / method: extract from start to body
            let start = node.start_byte();
            let end = node
                .child_by_field_name("body")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());

            source[start..end.min(source.len())].trim().to_string()
        }
    }
}

/// Extract parameters from C++ function/method definitions.
/// The parameter_list is a child of the function_declarator inside the declarator field.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let declarator = match node.child_by_field_name("declarator") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let params_node = match find_child_by_kind(&declarator, "parameter_list") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter_declaration"
            || child.kind() == "optional_parameter_declaration"
        {
            let type_ann = child
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());

            let param_name = child
                .child_by_field_name("declarator")
                .and_then(|n| {
                    // The declarator might be an identifier, pointer_declarator, or reference_declarator
                    if n.kind() == "identifier" {
                        n.utf8_text(source.as_bytes()).ok()
                    } else {
                        // For pointer/reference declarators, find the nested identifier
                        find_child_by_kind(&n, "identifier")
                            .and_then(|id| id.utf8_text(source.as_bytes()).ok())
                    }
                })
                .unwrap_or("");

            if !param_name.is_empty() {
                params.push(Parameter {
                    name: param_name.to_string(),
                    type_annotation: type_ann,
                });
            }
        }
    }

    params
}

/// Extract return type from the `type` field of function_definition.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("type")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Post-process C++ symbols.
/// For qualified_identifier names (e.g., `ClassName::method`), extract the method name
/// and set the parent from the qualifier.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    // Handle qualified identifiers like ClassName::method_name
    if sym.name.contains("::") {
        let parts: Vec<&str> = sym.name.rsplitn(2, "::").collect();
        if parts.len() == 2 {
            let method_name = parts[0];
            let qualifier = parts[1];
            // Only set parent if not already set
            if sym.parent_symbol.is_none() {
                sym.parent_symbol = Some(qualifier.to_string());
            }
            sym.name = method_name.to_string();
            // Qualified definitions outside class body are methods
            sym.kind = SymbolKind::Method;
        }
    }

    // Resolve parent from AST if not already set (for methods inside class/struct bodies)
    if sym.parent_symbol.is_none() {
        sym.parent_symbol = resolve_parent(node, source);
    }

    Some(sym)
}

/// Return C++ language hooks.
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
