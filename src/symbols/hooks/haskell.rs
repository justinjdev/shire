use super::{LanguageHooks, Parameter, SymbolInfo, SymbolKind, find_ancestor, node_text};
use tree_sitter::Node;

/// For methods inside a class declaration, resolve the class name.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    // signature -> class_declarations -> class
    let class_node = find_ancestor(node, "class")?;
    class_node
        .child_by_field_name("name")
        .and_then(|n| node_text(&n, source))
        .map(|s| s.to_string())
}

/// Find the type signature for a function by scanning preceding siblings.
///
/// In Haskell, type signatures are separate declarations that precede the function:
///   foo :: Int -> String
///   foo x = show x
/// In the AST these are sibling nodes under `declarations`. We scan backwards from the
/// `function` node for a `signature` sibling whose first `variable` child matches the name.
fn find_type_signature<'a>(node: &Node<'a>, source: &'a str, name: &str) -> Option<&'a str> {
    let mut sibling = node.prev_named_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "signature" {
            // The function name is the first `variable` child of the signature
            if let Some(sig_name) = first_variable_child(&sib)
                && node_text(&sig_name, source) == Some(name)
            {
                return source.get(sib.start_byte()..sib.end_byte());
            }
            // Stop scanning once we hit a non-matching signature
            return None;
        }
        // Skip over other function equations with the same name
        if sib.kind() == "function" {
            sibling = sib.prev_named_sibling();
            continue;
        }
        break;
    }
    None
}

/// Find the first child node of type `variable`.
fn first_variable_child<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "variable" {
            return Some(child);
        }
    }
    None
}

/// Build signature for Haskell symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => {
            // Try to find a preceding type signature
            if let Some(sig_text) = find_type_signature(node, source, name) {
                return sig_text.to_string();
            }
            // Fall back to source span up to the first match (equation body)
            let start = node.start_byte();
            let end = node
                .child_by_field_name("match")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            let sig = source[start..end.min(source.len())].trim();
            // Trim trailing '=' if present
            sig.trim_end_matches('=').trim().to_string()
        }
        SymbolKind::Method => {
            // Class method: signature node itself — use full text
            let text = node_text(node, source).unwrap_or(name);
            text.to_string()
        }
        SymbolKind::Trait => {
            // class declaration
            let start = node.start_byte();
            let end = node
                .child_by_field_name("declarations")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            let sig = source[start..end.min(source.len())].trim();
            sig.trim_end_matches("where").trim().to_string()
        }
        _ => {
            // data_type, newtype, type_synomym
            let start = node.start_byte();
            let end = node.end_byte();
            let full = &source[start..end.min(source.len())];
            // Take first line only for multi-line declarations
            let first_line = full.lines().next().unwrap_or(full);
            // Trim deriving clauses
            if let Some(idx) = first_line.find("deriving") {
                first_line[..idx].trim().to_string()
            } else {
                first_line.trim().to_string()
            }
        }
    }
}

/// Extract parameters from Haskell function patterns.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let patterns_node = match node.child_by_field_name("patterns") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    for i in 0..patterns_node.child_count() {
        let child = patterns_node.child(i).unwrap();
        if let Some(text) = node_text(&child, source) {
            let text = text.trim();
            if !text.is_empty() {
                params.push(Parameter {
                    name: text.to_string(),
                    type_annotation: None,
                });
            }
        }
    }
    params
}

/// Extract return type from a Haskell function's type signature.
/// For `foo :: Int -> String -> Bool`, the return type is `Bool`.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    let name_node = first_variable_child(node)?;
    let fn_name = node_text(&name_node, source)?;
    let sig_text = find_type_signature(node, source, fn_name)?;

    // Parse "name :: A -> B -> C" to extract "C"
    let after_colons = sig_text.split_once("::")?.1.trim();
    // Split on " -> " and take the last segment
    let parts: Vec<&str> = split_arrow_type(after_colons);
    if parts.len() > 1 {
        Some(parts.last()?.trim().to_string())
    } else {
        None
    }
}

/// Split a Haskell type on top-level arrows, respecting parentheses.
fn split_arrow_type(ty: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let bytes = ty.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'-' if depth == 0 && i + 1 < bytes.len() && bytes[i + 1] == b'>' => {
                parts.push(ty[start..i].trim());
                i += 2;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        parts.push(ty[start..].trim());
    }
    parts
}

/// Post-process: reclassify types and deduplicate multi-equation functions.
///
/// In Haskell, a function with pattern matching has multiple equations, each parsed as
/// a separate `function` node. We skip duplicates by checking if the previous sibling
/// is also a `function` with the same name.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    match node.kind() {
        "data_type" | "newtype" => {
            sym.kind = SymbolKind::Struct;
        }
        "function" => {
            // Deduplicate multi-equation functions: skip if a preceding sibling
            // is a `function` with the same name
            if let Some(prev) = node.prev_named_sibling()
                && prev.kind() == "function"
                && let Some(prev_name) = first_variable_child(&prev)
                && node_text(&prev_name, source) == Some(&sym.name)
            {
                return None;
            }
        }
        _ => {}
    }
    Some(sym)
}

/// Return Haskell language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: None,
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: Some(post_process),
        reference_hooks: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_arrow_type_simple() {
        let parts = split_arrow_type("Int -> String -> Bool");
        assert_eq!(parts, vec!["Int", "String", "Bool"]);
    }

    #[test]
    fn test_split_arrow_type_with_parens() {
        let parts = split_arrow_type("(Int -> String) -> Bool");
        assert_eq!(parts, vec!["(Int -> String)", "Bool"]);
    }

    #[test]
    fn test_split_arrow_type_single() {
        let parts = split_arrow_type("Int");
        assert_eq!(parts, vec!["Int"]);
    }
}
