use super::{find_ancestor, find_child_by_kind, node_text, LanguageHooks, Parameter, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Valid public definition keywords for call nodes.
const PUBLIC_DEF_KEYWORDS: &[&str] = &["def", "defmacro", "defguard", "defdelegate", "defmodule", "defprotocol"];

/// Valid attribute names for unary_operator (@attr) nodes.
const ATTR_KEYWORDS: &[&str] = &["type", "opaque", "callback"];

/// Get the target identifier text of a call node.
fn call_target_text<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    node.child_by_field_name("target")
        .and_then(|t| node_text(&t, source))
}

/// For unary_operator nodes (@type, @callback), get the attribute name.
fn attr_name<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    node.child_by_field_name("operand")
        .and_then(|op| op.child_by_field_name("target"))
        .and_then(|t| node_text(&t, source))
}

/// Filter: include only valid public definitions.
fn is_visible(node: &Node, source: &str) -> bool {
    match node.kind() {
        "call" => call_target_text(node, source)
            .is_some_and(|kw| PUBLIC_DEF_KEYWORDS.contains(&kw)),
        "unary_operator" => attr_name(node, source)
            .is_some_and(|name| ATTR_KEYWORDS.contains(&name)),
        _ => false,
    }
}

/// Resolve parent module/protocol name by walking up to enclosing defmodule/defprotocol.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    let do_block = find_ancestor(node, "do_block")?;
    let parent_call = do_block.parent()?;
    if parent_call.kind() != "call" {
        return None;
    }
    let keyword = call_target_text(&parent_call, source)?;
    if keyword != "defmodule" && keyword != "defprotocol" {
        return None;
    }
    let args = find_child_by_kind(&parent_call, "arguments")?;
    find_child_by_kind(&args, "alias")
        .and_then(|a| node_text(&a, source))
        .map(|s| s.to_string())
}

/// Find the inner call node from a def/defmacro's arguments.
/// Handles both direct call children and guard clauses (binary_operator wrapping call).
fn find_inner_call<'a>(outer_args: &'a Node<'a>) -> Option<Node<'a>> {
    find_child_by_kind(outer_args, "call").or_else(|| {
        find_child_by_kind(outer_args, "binary_operator")
            .and_then(|bo| bo.child_by_field_name("left"))
            .filter(|n| n.kind() == "call")
    })
}

/// Build signature string for Elixir symbols.
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    match node.kind() {
        "call" => {
            let keyword = call_target_text(node, source).unwrap_or("def");
            match keyword {
                "defmodule" | "defprotocol" => format!("{keyword} {name} do"),
                _ => {
                    let outer_args = find_child_by_kind(node, "arguments");
                    let inner_call = outer_args.as_ref().and_then(find_inner_call);

                    if let Some(inner) = inner_call
                        && let Some(args_node) = find_child_by_kind(&inner, "arguments") {
                            let args_text = node_text(&args_node, source).unwrap_or("()");
                            let trimmed = args_text.trim();
                            if let Some(inner_text) =
                                trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
                            {
                                return format!("{keyword} {name}({})", inner_text.trim());
                            }
                        }
                    format!("{keyword} {name}")
                }
            }
        }
        "unary_operator" => {
            let attr = attr_name(node, source).unwrap_or("type");
            if let Some(operand) = node.child_by_field_name("operand")
                && let Some(args) = find_child_by_kind(&operand, "arguments") {
                    let text = node_text(&args, source).unwrap_or(name);
                    return format!("@{attr} {}", text.trim());
                }
            format!("@{attr} {name}")
        }
        _ => name.to_string(),
    }
}

/// Extract parameters from def/defmacro function arguments.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let outer_args = match find_child_by_kind(node, "arguments") {
        Some(a) => a,
        None => return Vec::new(),
    };

    let inner_call = match find_inner_call(&outer_args) {
        Some(c) => c,
        None => return Vec::new(),
    };

    let inner_args = match find_child_by_kind(&inner_call, "arguments") {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    for i in 0..inner_args.child_count() {
        let child = inner_args.child(i).unwrap();
        match child.kind() {
            "identifier" => {
                if let Some(name) = node_text(&child, source) {
                    params.push(Parameter {
                        name: name.to_string(),
                        type_annotation: None,
                    });
                }
            }
            "binary_operator" => {
                // Default value: name \\ default
                if let Some(left) = child.child_by_field_name("left")
                    && left.kind() == "identifier"
                        && let Some(name) = node_text(&left, source) {
                            params.push(Parameter {
                                name: name.to_string(),
                                type_annotation: None,
                            });
                        }
            }
            _ => {}
        }
    }
    params
}

/// Extract type parameters from @callback signature.
fn extract_callback_params(call_node: &Node, source: &str) -> Vec<Parameter> {
    let args = match find_child_by_kind(call_node, "arguments") {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    for i in 0..args.child_count() {
        let child = args.child(i).unwrap();
        match child.kind() {
            "(" | ")" | "," => continue,
            _ => {
                if let Some(text) = node_text(&child, source) {
                    params.push(Parameter {
                        name: format!("arg{}", params.len()),
                        type_annotation: Some(text.trim().to_string()),
                    });
                }
            }
        }
    }
    params
}

/// Post-process: reclassify symbol kinds and extract return types for @type/@callback.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    match node.kind() {
        "call" => {
            let keyword = call_target_text(node, source)?;
            if keyword == "defprotocol" {
                sym.kind = SymbolKind::Interface;
            }
        }
        "unary_operator" => {
            let attr = attr_name(node, source)?;
            if let Some(operand) = node.child_by_field_name("operand")
                && let Some(args) = find_child_by_kind(&operand, "arguments")
                    && let Some(binop) = find_child_by_kind(&args, "binary_operator") {
                        let return_type = binop
                            .child_by_field_name("right")
                            .and_then(|r| node_text(&r, source))
                            .map(|s| s.trim().to_string());
                        sym.return_type = return_type;

                        if attr == "callback" {
                            sym.kind = SymbolKind::Method;
                            if let Some(left) = binop.child_by_field_name("left")
                                && left.kind() == "call" {
                                    sym.parameters = Some(extract_callback_params(&left, source));
                                }
                        }
                    }
        }
        _ => {}
    }
    Some(sym)
}

pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: Some(is_visible),
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: None,
        post_process: Some(post_process),
        reference_hooks: None,
    }
}
