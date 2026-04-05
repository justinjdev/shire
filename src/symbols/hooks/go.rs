use super::{field_text, node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Go visibility: only symbols starting with an uppercase letter are exported.
fn is_visible(node: &Node, source: &str) -> bool {
    let name = field_text(node, "name", source);
    name.is_some_and(|n| {
        n.chars().next().is_some_and(|c| c.is_uppercase())
    })
}

/// For method_declaration, extract receiver type (strip leading `*`).
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "method_declaration" {
        return None;
    }

    let receiver = node.child_by_field_name("receiver")?;
    for i in 0..receiver.child_count() {
        let child = receiver.child(i).unwrap();
        if child.kind() == "parameter_declaration"
            && let Some(type_node) = child.child_by_field_name("type") {
                let type_text = node_text(&type_node, source)?;
                return Some(type_text.trim_start_matches('*').to_string());
            }
    }
    None
}

/// Build signature for Go symbols.
/// For functions/methods: source span from node start to end of result (or parameters).
/// For type specs: "type Name kind" with _type stripped from the kind.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Struct | SymbolKind::Interface | SymbolKind::Type => {
            // type_spec node — get the type child's kind
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_kind = type_node.kind().replace("_type", "");
                format!("type {} {}", name, type_kind)
            } else {
                format!("type {}", name)
            }
        }
        _ => {
            // function_declaration or method_declaration
            let start = node.start_byte();
            let end = node
                .child_by_field_name("result")
                .map(|n| n.end_byte())
                .or_else(|| node.child_by_field_name("parameters").map(|n| n.end_byte()))
                .unwrap_or(node.end_byte());

            let actual_end = node
                .child_by_field_name("body")
                .map(|n| n.start_byte())
                .unwrap_or(end);

            source[start..actual_end.min(source.len())].trim().to_string()
        }
    }
}

/// Extract parameters from Go function/method declarations.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name("parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "parameter_declaration" {
            let type_ann = child
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());

            if let Some(name_node) = child.child_by_field_name("name") {
                params.push(Parameter {
                    name: name_node
                        .utf8_text(source.as_bytes())
                        .unwrap_or("")
                        .to_string(),
                    type_annotation: type_ann,
                });
            }
        }
    }

    params
}

/// Extract return type from the `result` field.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("result")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Post-process: reclassify type_spec nodes to Struct/Interface based on their type child.
fn post_process(mut sym: SymbolInfo, node: &Node, _source: &str) -> Option<SymbolInfo> {
    if sym.kind == SymbolKind::Type && node.kind() == "type_spec"
        && let Some(type_node) = node.child_by_field_name("type") {
            sym.kind = match type_node.kind() {
                "struct_type" => SymbolKind::Struct,
                "interface_type" => SymbolKind::Interface,
                _ => SymbolKind::Type,
            };
        }
    Some(sym)
}

/// Return Go language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        enclosing_ancestors: &[
            "function_declaration",
            "method_declaration",
        ],
        reference_stoplist: &[
            "true", "false", "nil", "iota",
            "make", "new", "len", "cap", "append", "copy", "delete",
            "print", "println", "panic", "recover",
            "min", "max", "clear",
            "int", "int32", "int64", "uint", "uint32", "uint64",
            "string", "bool", "byte", "rune", "float32", "float64",
            "error", "any",
            "complex", "real", "imag", "close",
            "uintptr", "complex64", "complex128", "comparable",
        ],
    }
}
