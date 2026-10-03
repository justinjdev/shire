use super::{
    LanguageHooks, Parameter, ReferenceHooks, SymbolInfo, SymbolKind, Visibility, find_ancestor,
    find_child_by_kind, modifier_text, narrowest, node_text,
};
use tree_sitter::Node;

/// TypeScript/JavaScript visibility.
///
/// A module-level declaration is public when it is exported — wrapped in an
/// `export` statement, or named in an `export { ... }` clause of the same file
/// — and private to its module otherwise. (CommonJS `module.exports = ...` is
/// not recognised, so a CommonJS module's functions are recorded as private.)
///
/// A class method takes its own `private`/`protected` modifier (or a
/// `#private` name), narrowed by its class's visibility.
fn visibility(node: &Node, source: &str) -> Visibility {
    if node.kind() != "method_definition" {
        return declaration_visibility(node, source);
    }
    let own = if node
        .child_by_field_name("name")
        .and_then(|n| node_text(&n, source))
        .is_some_and(|name| name.starts_with('#'))
    {
        Visibility::Private
    } else {
        match modifier_text(node, source, "accessibility_modifier") {
            Some("private") => Visibility::Private,
            Some("protected") => Visibility::Protected,
            _ => Visibility::Public,
        }
    };
    match find_ancestor(node, "class_declaration") {
        Some(class) => narrowest(own, declaration_visibility(&class, source)),
        None => own,
    }
}

/// Whether a module-level declaration is exported. A `variable_declarator` is
/// judged by its enclosing `lexical_declaration`.
fn declaration_visibility(node: &Node, source: &str) -> Visibility {
    let decl = if node.kind() == "variable_declarator" {
        node.parent().unwrap_or(*node)
    } else {
        *node
    };
    let Some(parent) = decl.parent() else {
        return Visibility::Public;
    };
    if parent.kind() == "export_statement" {
        return Visibility::Public;
    }
    let name = node
        .child_by_field_name("name")
        .and_then(|n| node_text(&n, source));
    if parent.kind() == "program"
        && let Some(name) = name
        && named_in_export_clause(&parent, name, source)
    {
        return Visibility::Public;
    }
    Visibility::Private
}

/// True if `program` has an `export { ..., name, ... }` clause (with or
/// without `as`) that exports the local binding `name`.
fn named_in_export_clause(program: &Node, name: &str, source: &str) -> bool {
    for i in 0..program.named_child_count() {
        let stmt = program.named_child(i).unwrap();
        if stmt.kind() != "export_statement" || stmt.child_by_field_name("source").is_some() {
            // `export { x } from "./m"` re-exports another module's binding.
            continue;
        }
        let Some(clause) = find_child_by_kind(&stmt, "export_clause") else {
            continue;
        };
        for j in 0..clause.named_child_count() {
            let spec = clause.named_child(j).unwrap();
            if spec.kind() == "export_specifier"
                && spec
                    .child_by_field_name("name")
                    .and_then(|n| node_text(&n, source))
                    == Some(name)
            {
                return true;
            }
        }
    }
    false
}

/// Resolve parent symbol name for methods inside classes.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    if node.kind() != "method_definition" {
        return None;
    }

    let class_node = find_ancestor(node, "class_declaration")?;
    let name_node = class_node.child_by_field_name("name")?;
    name_node
        .utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.to_string())
}

/// Build signature string for TypeScript/JavaScript symbols.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => build_function_signature(node, source),
        SymbolKind::Method => build_method_signature(node, source, name),
        SymbolKind::Class => format!("class {}", name),
        SymbolKind::Interface => format!("interface {}", name),
        SymbolKind::Enum => format!("enum {}", name),
        SymbolKind::Type => {
            // For type aliases, return first line of the node text
            node_text(node, source)
                .map(|t| t.lines().next().unwrap_or(t).to_string())
                .unwrap_or_else(|| format!("type {}", name))
        }
        SymbolKind::Constant => {
            // For constants, get the lexical_declaration parent text (first line)
            if let Some(parent) = node.parent()
                && parent.kind() == "lexical_declaration"
            {
                return node_text(&parent, source)
                    .map(|t| t.lines().next().unwrap_or(t).to_string())
                    .unwrap_or_else(|| format!("const {}", name));
            }
            node_text(node, source)
                .map(|t| t.lines().next().unwrap_or(t).to_string())
                .unwrap_or_else(|| format!("const {}", name))
        }
        _ => name.to_string(),
    }
}

/// Build function signature: source span from start to end of return_type or parameters.
fn build_function_signature(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let end = node
        .child_by_field_name("return_type")
        .map(|n| n.end_byte())
        .or_else(|| node.child_by_field_name("parameters").map(|n| n.end_byte()))
        .unwrap_or(node.end_byte());

    let text = &source[start..end.min(source.len())];
    text.lines()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Build method signature: `name(params): return_type`.
fn build_method_signature(node: &Node, source: &str, name: &str) -> String {
    let params = node
        .child_by_field_name("parameters")
        .and_then(|n| node_text(&n, source))
        .unwrap_or("()");
    let ret = node
        .child_by_field_name("return_type")
        .and_then(|n| unwrap_type_annotation(&n, source))
        .map(|r| format!(": {}", r))
        .unwrap_or_default();
    format!("{}{}{}", name, params, ret)
}

/// Unwrap a type from a type_annotation node, skipping the colon.
fn unwrap_type_annotation(node: &Node, source: &str) -> Option<String> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() != ":" {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
    }
    node.utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.trim_start_matches(": ").to_string())
}

/// Extract parameters from TypeScript/JavaScript function/method nodes.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name("parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        match child.kind() {
            "required_parameter" | "optional_parameter" => {
                let name = child
                    .child_by_field_name("pattern")
                    .or_else(|| child.child_by_field_name("name"))
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("")
                    .to_string();

                let type_ann = child
                    .child_by_field_name("type")
                    .and_then(|n| unwrap_type_annotation(&n, source));

                if !name.is_empty() {
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
            "identifier" => {
                // JS-style simple parameters
                if let Ok(name) = child.utf8_text(source.as_bytes()) {
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

/// Extract return type hook.
fn extract_return_type(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("return_type")
        .and_then(|n| unwrap_type_annotation(&n, source))
}

/// Post-process symbols.
///
/// - Filters class/interface/type_alias declarations that are neither
///   exported nor at module level (e.g. a class declared inside a function
///   body). These come from supplementary suppression-only query patterns
///   whose sole purpose is to seed def_name_ranges (run before post_process)
///   so the bare `(type_identifier) @reference.type` pattern does not emit a
///   self-ref at the declaration line.
/// - For `variable_declarator` nodes (Constant kind): extract name from the
///   variable_declarator's name field, and set signature from the parent
///   lexical_declaration's first line.
/// - For `type_alias_declaration` nodes: set signature to first line of node
///   text.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    if matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "type_alias_declaration"
    ) {
        let module_level = node
            .parent()
            .is_some_and(|p| matches!(p.kind(), "export_statement" | "program"));
        if !module_level {
            return None;
        }
    }

    if sym.kind == SymbolKind::Constant && node.kind() == "variable_declarator" {
        // Name is already captured by @name on the variable_declarator's name field.
        // Set signature from the parent lexical_declaration.
        if let Some(parent) = node.parent()
            && parent.kind() == "lexical_declaration"
        {
            sym.signature =
                node_text(&parent, source).map(|t| t.lines().next().unwrap_or(t).to_string());
        }
    }

    if sym.kind == SymbolKind::Type && node.kind() == "type_alias_declaration" {
        sym.signature = node_text(node, source).map(|t| t.lines().next().unwrap_or(t).to_string());
    }

    Some(sym)
}

/// Return the language hooks for TypeScript and JavaScript.
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
                "function_declaration",
                "method_definition",
                "class_declaration",
                "interface_declaration",
                "function_expression",
                "method_signature",
                // arrow_function is a distinct callable kind in TS — without
                // it refs captured inside arrow bodies get no enclosing_symbol.
                "arrow_function",
            ],
            // Keep only literals/keywords and TS-reserved type keywords. Global
            // classes like String/Number/Array/Promise/Error ARE user-definable
            // (you can write `class Array { ... }`), so stoplisting them turns
            // any repo type with those names into a permanent false negative.
            reference_stoplist: &[
                "true",
                "false",
                "null",
                "undefined",
                "this",
                "super",
                "string",
                "number",
                "boolean",
                "any",
                "unknown",
                "never",
                "void",
            ],
        }),
    }
}
