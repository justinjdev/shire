use super::{find_child_by_kind, node_text, LanguageHooks, Parameter, SymbolKind};
use tree_sitter::Node;

/// Lua visibility: all symbols are visible (Lua has no access modifiers).
fn is_visible(_node: &Node, _source: &str) -> bool {
    true
}

/// Resolve the parent (table/module) name for methods and module functions.
///
/// For `function M:foo()` → parent is "M" (from method_index_expression.table)
/// For `function M.foo()` → parent is "M" (from dot_index_expression.table)
/// For assignment-style `M.foo = function()` → parent is "M" (from dot_index_expression in variable_list)
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "function_declaration" => {
            let name_node = node.child_by_field_name("name")?;
            match name_node.kind() {
                "method_index_expression" | "dot_index_expression" => {
                    let table = name_node.child_by_field_name("table")?;
                    node_text(&table, source).map(|s| s.to_string())
                }
                _ => None,
            }
        }
        "assignment_statement" => {
            // M.foo = function() end
            let var_list = find_child_by_kind(node, "variable_list")?;
            let first_var = var_list.child_by_field_name("name")?;
            if first_var.kind() == "dot_index_expression" {
                let table = first_var.child_by_field_name("table")?;
                node_text(&table, source).map(|s| s.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Build signature string for Lua symbols.
///
/// Functions/methods: `function name(params)` or `function Class:method(params)`
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match node.kind() {
        "function_declaration" => {
            let name_node = node.child_by_field_name("name");
            let full_name = name_node
                .and_then(|n| node_text(&n, source))
                .unwrap_or(name);

            let params = node
                .child_by_field_name("parameters")
                .and_then(|n| node_text(&n, source))
                .unwrap_or("()");

            format!("function {}{}", full_name, params)
        }
        "assignment_statement" => {
            // Find the function_definition to get parameters
            let el = find_child_by_kind(node, "expression_list");
            let fd = el.as_ref().and_then(|el| find_child_by_kind(el, "function_definition"));
            let params = fd
                .as_ref()
                .and_then(|fd| fd.child_by_field_name("parameters"))
                .and_then(|n| node_text(&n, source))
                .unwrap_or("()");

            // Get the full variable name (e.g., M.foo or just foo)
            let vl = find_child_by_kind(node, "variable_list");
            let var_name = vl
                .as_ref()
                .and_then(|vl| vl.child_by_field_name("name"))
                .and_then(|n| node_text(&n, source))
                .unwrap_or(name);

            format!("function {}{}", var_name, params)
        }
        _ => match kind {
            SymbolKind::Method => format!("function {}", name),
            _ => format!("function {}", name),
        },
    }
}

/// Collect parameter identifiers from a parameters node, filtering out `self`.
fn collect_params(params_node: &Node, source: &str) -> Vec<Parameter> {
    let mut params = Vec::new();
    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        if child.kind() == "identifier" {
            let pname = child
                .utf8_text(source.as_bytes())
                .unwrap_or("")
                .to_string();
            if !pname.is_empty() && pname != "self" {
                params.push(Parameter {
                    name: pname,
                    type_annotation: None, // Lua is dynamically typed
                });
            }
        }
    }
    params
}

/// Extract parameters from a Lua function declaration or assignment.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    match node.kind() {
        "function_declaration" => {
            match node.child_by_field_name("parameters") {
                Some(p) => collect_params(&p, source),
                None => Vec::new(),
            }
        }
        "assignment_statement" => {
            // Navigate: assignment_statement > expression_list > function_definition > parameters
            let el = match find_child_by_kind(node, "expression_list") {
                Some(n) => n,
                None => return Vec::new(),
            };
            let fd = match find_child_by_kind(&el, "function_definition") {
                Some(n) => n,
                None => return Vec::new(),
            };
            match fd.child_by_field_name("parameters") {
                Some(p) => collect_params(&p, source),
                None => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// Lua has no type annotations — always returns None.
fn extract_return_type(_node: &Node, _source: &str) -> Option<String> {
    None
}

/// Return the language hooks for Lua.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: Some(extract_return_type),
        post_process: None,
        reference_hooks: None,
    }
}
