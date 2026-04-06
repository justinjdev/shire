use super::{find_ancestor, find_child_by_kind, node_text, LanguageHooks, Parameter, ReferenceHooks, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Resolve the parent class or module for a method.
fn resolve_parent(node: &Node, source: &str) -> Option<String> {
    // For singleton_method, the parent is the enclosing class/module
    // For method, the parent is also the enclosing class/module
    if let Some(class_node) = find_ancestor(node, "class") {
        return class_node
            .child_by_field_name("name")
            .and_then(|n| node_text(&n, source))
            .map(|s| s.to_string());
    }
    if let Some(mod_node) = find_ancestor(node, "module") {
        return mod_node
            .child_by_field_name("name")
            .and_then(|n| node_text(&n, source))
            .map(|s| s.to_string());
    }
    None
}

/// Build signature for Ruby symbols.
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    match node.kind() {
        "class" => {
            // Check for superclass — the field includes the `<` token, so find
            // the constant child inside the superclass node
            if let Some(superclass) = node.child_by_field_name("superclass") {
                if let Some(sc_const) = find_child_by_kind(&superclass, "constant")
                    && let Some(sc_name) = node_text(&sc_const, source) {
                        return format!("class {name} < {sc_name}");
                    }
                // Fallback to scope_resolution for namespaced superclasses
                if let Some(sc_name) = node_text(&superclass, source) {
                    let sc_name = sc_name.trim_start_matches('<').trim();
                    return format!("class {name} < {sc_name}");
                }
            }
            format!("class {name}")
        }
        "module" => format!("module {name}"),
        "singleton_method" => {
            // def self.name(params)
            let params = format_params(node, source);
            if params.is_empty() {
                format!("def self.{name}")
            } else {
                format!("def self.{name}({params})")
            }
        }
        "method" => {
            let params = format_params(node, source);
            if params.is_empty() {
                format!("def {name}")
            } else {
                format!("def {name}({params})")
            }
        }
        _ => format!("def {name}"),
    }
}

/// Format parameter list from a method/singleton_method node.
fn format_params(node: &Node, source: &str) -> String {
    let params_node = match find_child_by_kind(node, "method_parameters") {
        Some(n) => n,
        None => return String::new(),
    };
    // Get the text between the parens
    let text = node_text(&params_node, source).unwrap_or("");
    let trimmed = text.trim();
    // Strip surrounding parens
    if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        inner.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// Extract parameters from a Ruby method.
fn extract_parameters(node: &Node, source: &str) -> Vec<Parameter> {
    let params_node = match find_child_by_kind(node, "method_parameters") {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();
    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        match child.kind() {
            "identifier" => {
                if let Some(name) = node_text(&child, source) {
                    params.push(Parameter {
                        name: name.to_string(),
                        type_annotation: None,
                    });
                }
            }
            "splat_parameter" => {
                // *args
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = node_text(&name_node, source) {
                        params.push(Parameter {
                            name: name.to_string(),
                            type_annotation: Some("*".to_string()),
                        });
                    }
            }
            "hash_splat_parameter" => {
                // **opts
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = node_text(&name_node, source) {
                        params.push(Parameter {
                            name: name.to_string(),
                            type_annotation: Some("**".to_string()),
                        });
                    }
            }
            "block_parameter" => {
                // &block
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = node_text(&name_node, source) {
                        params.push(Parameter {
                            name: name.to_string(),
                            type_annotation: Some("&".to_string()),
                        });
                    }
            }
            "keyword_parameter" => {
                // name: or name: default
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = node_text(&name_node, source) {
                        params.push(Parameter {
                            name: name.to_string(),
                            type_annotation: None,
                        });
                    }
            }
            "optional_parameter" => {
                // name = default
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = node_text(&name_node, source) {
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

/// Post-process: reclassify methods inside classes/modules as Method kind.
/// Singleton methods stay as Function.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    match node.kind() {
        "method" => {
            if resolve_parent(node, source).is_some() {
                sym.kind = SymbolKind::Method;
            }
        }
        "singleton_method" => {
            // Class methods stay as Function
            sym.kind = SymbolKind::Function;
        }
        "module" => {
            // Modules reported as Class (matching existing behavior)
            sym.kind = SymbolKind::Class;
        }
        _ => {}
    }
    Some(sym)
}

/// Return Ruby language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        is_visible: None,
        resolve_parent: Some(resolve_parent),
        build_signature: Some(build_signature),
        extract_parameters: Some(extract_parameters),
        extract_return_type: None,
        post_process: Some(post_process),
        reference_hooks: Some(ReferenceHooks {
            enclosing_ancestors: &[
                "method",
                "singleton_method",
                "class",
                "module",
            ],
            reference_stoplist: &[
                "true", "false", "nil", "self",
                "puts", "print", "p", "pp",
                "String", "Integer", "Float", "Array", "Hash", "Symbol", "NilClass",
                "Object", "Class", "Module",
                // Mixin/import methods: captured separately as @reference.impl
                // and @reference.import, so suppress their redundant Call refs.
                "include", "prepend", "extend",
                "require", "require_relative", "load",
            ],
        }),
    }
}
