use super::lang_spec::*;
use super::{Parameter, SymbolInfo, SymbolKind};
use tree_sitter::{Node, Parser};

/// Extract symbols from source code using a language specification.
pub fn extract(spec: &LanguageSpec, source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let mut parser = Parser::new();
    if parser.set_language(&(spec.ts_language)()).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut symbols = Vec::new();
    let root = tree.root_node();

    for i in 0..root.child_count() {
        let node = root.child(i).unwrap();
        let kind = node.kind();

        if let Some(mapping) = spec.definition_nodes.iter().find(|m| m.node_type == kind) {
            match mapping.symbol_kind {
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface => {
                    extract_class_like(spec, source, file_path, &node, mapping.symbol_kind, &mut symbols);
                }
                _ => {
                    if check_visibility(spec, &node, source) {
                        if let Some(sym) = extract_definition(spec, source, file_path, &node, mapping.symbol_kind, None) {
                            symbols.push(sym);
                        }
                    }
                }
            }
        }
    }

    symbols
}

fn extract_definition(
    spec: &LanguageSpec,
    source: &str,
    file_path: &str,
    node: &Node,
    kind: SymbolKind,
    parent: Option<&str>,
) -> Option<SymbolInfo> {
    let name = node
        .child_by_field_name(spec.name_field)?
        .utf8_text(source.as_bytes())
        .ok()?
        .to_string();

    let line = node.start_position().row + 1;
    let params = extract_parameters(spec, source, node);
    let return_type = extract_return_type(spec, source, node);
    let signature = build_signature(spec, source, node, &name, kind);

    // Filter self params for methods
    let filtered_params = if parent.is_some() {
        params
            .into_iter()
            .filter(|p| !spec.method_spec.as_ref().map_or(false, |ms| ms.self_param_names.contains(&p.name.as_str())))
            .collect()
    } else {
        params
    };

    Some(SymbolInfo {
        name,
        kind,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: parent.map(|s| s.to_string()),
        return_type,
        parameters: if matches!(kind, SymbolKind::Function | SymbolKind::Method) {
            Some(filtered_params)
        } else {
            None
        },
    })
}

fn extract_class_like(
    spec: &LanguageSpec,
    source: &str,
    file_path: &str,
    node: &Node,
    kind: SymbolKind,
    symbols: &mut Vec<SymbolInfo>,
) {
    if !check_visibility(spec, node, source) {
        return;
    }

    let class_name = match node
        .child_by_field_name(spec.name_field)
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
    {
        Some(name) => name.to_string(),
        None => return,
    };

    let line = node.start_position().row + 1;
    let signature = build_signature(spec, source, node, &class_name, kind);

    symbols.push(SymbolInfo {
        name: class_name.clone(),
        kind,
        signature: Some(signature),
        file_path: file_path.to_string(),
        line,
        visibility: "public".to_string(),
        parent_symbol: None,
        return_type: None,
        parameters: None,
    });

    // Extract methods from class body
    if let Some(method_spec) = &spec.method_spec {
        if let Some(body) = node.child_by_field_name(method_spec.body_field) {
            for i in 0..body.child_count() {
                let child = body.child(i).unwrap();
                if !method_spec.method_node_kinds.contains(&child.kind()) {
                    continue;
                }

                let method_name = match child
                    .child_by_field_name(spec.name_field)
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                {
                    Some(name) => name.to_string(),
                    None => continue,
                };

                // Check method visibility
                if let Some(vis) = &method_spec.visibility {
                    match vis {
                        MethodVisibility::SkipPrefix(prefix) => {
                            // Allow constructor even if it matches the prefix
                            let is_constructor = spec.constructor_name == Some(method_name.as_str());
                            if !is_constructor && method_name.starts_with(prefix) {
                                continue;
                            }
                        }
                        MethodVisibility::SkipAccessModifier {
                            node_kind,
                            blocked_values,
                        } => {
                            if has_blocked_modifier(&child, source, node_kind, blocked_values) {
                                continue;
                            }
                        }
                        MethodVisibility::SkipNamePrefix(ch) => {
                            if method_name.starts_with(*ch) {
                                continue;
                            }
                        }
                    }
                }

                if let Some(mut sym) =
                    extract_definition(spec, source, file_path, &child, SymbolKind::Method, Some(&class_name))
                {
                    symbols.push(sym);
                }
            }
        }
    }
}

fn check_visibility(spec: &LanguageSpec, node: &Node, source: &str) -> bool {
    match &spec.visibility {
        VisibilityRule::AllPublic => true,
        VisibilityRule::UppercaseExported => {
            let name = node
                .child_by_field_name(spec.name_field)
                .and_then(|n| n.utf8_text(source.as_bytes()).ok());
            match name {
                Some(n) => n.chars().next().map_or(false, |c| c.is_uppercase()),
                None => false,
            }
        }
        VisibilityRule::HasChildNode(kind) => {
            for i in 0..node.child_count() {
                if node.child(i).unwrap().kind() == *kind {
                    return true;
                }
            }
            false
        }
        VisibilityRule::NoAccessModifier {
            node_kind,
            blocked_values,
        } => !has_blocked_modifier(node, source, node_kind, blocked_values),
        VisibilityRule::ExportWrapped => {
            // This is handled at the dispatch level, not here
            true
        }
        VisibilityRule::NoPrefix(prefix) => {
            let name = node
                .child_by_field_name(spec.name_field)
                .and_then(|n| n.utf8_text(source.as_bytes()).ok());
            match name {
                Some(n) => !n.starts_with(prefix),
                None => false,
            }
        }
    }
}

fn has_blocked_modifier(
    node: &Node,
    source: &str,
    modifier_kind: &str,
    blocked_values: &[&str],
) -> bool {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == modifier_kind {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                if blocked_values.contains(&text) {
                    return true;
                }
            }
        }
    }
    false
}

fn extract_parameters(spec: &LanguageSpec, source: &str, node: &Node) -> Vec<Parameter> {
    let params_node = match node.child_by_field_name(spec.parameters_field) {
        Some(n) => n,
        None => return Vec::new(),
    };

    let mut params = Vec::new();

    for i in 0..params_node.child_count() {
        let child = params_node.child(i).unwrap();
        let child_kind = child.kind();

        // Skip self parameter node kinds
        if let Some(ms) = &spec.method_spec {
            if ms.self_param_kinds.contains(&child_kind) {
                continue;
            }
        }

        if let Some(param_spec) = spec.param_specs.iter().find(|ps| ps.kind == child_kind) {
            let name = extract_field_value(source, &child, param_spec.name_source);
            let type_ann = param_spec
                .type_source
                .and_then(|fs| extract_field_value(source, &child, fs));

            if let Some(name) = name {
                if !name.is_empty() {
                    params.push(Parameter {
                        name,
                        type_annotation: type_ann,
                    });
                }
            }
        }
    }

    params
}

fn extract_field_value(source: &str, node: &Node, field: FieldSource) -> Option<String> {
    match field {
        FieldSource::Field(name) => node
            .child_by_field_name(name)
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string()),
        FieldSource::Child(index) => node
            .child(index)
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string()),
        FieldSource::NodeText => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
    }
}

fn extract_return_type(spec: &LanguageSpec, source: &str, node: &Node) -> Option<String> {
    let field_name = spec.return_type_field?;
    let ret_node = node.child_by_field_name(field_name)?;

    if spec.return_type_unwrap_colon {
        // The return type node includes `:` — find the actual type inside
        for i in 0..ret_node.child_count() {
            let child = ret_node.child(i).unwrap();
            if child.kind() != ":" {
                return child.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
            }
        }
        // Fallback: trim the colon prefix
        ret_node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.trim_start_matches(": ").to_string())
    } else {
        ret_node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string())
    }
}

fn build_signature(
    spec: &LanguageSpec,
    source: &str,
    node: &Node,
    name: &str,
    kind: SymbolKind,
) -> String {
    match kind {
        SymbolKind::Function | SymbolKind::Method => match &spec.fn_signature {
            SignatureStyle::KeywordBased(keyword) => {
                let params_text = node
                    .child_by_field_name(spec.parameters_field)
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("()");
                let ret = extract_return_type(spec, source, node)
                    .map(|r| format!(" -> {}", r))
                    .unwrap_or_default();
                format!("{} {}{}{}", keyword, name, params_text, ret)
            }
            SignatureStyle::SourceSpan => {
                let start = node.start_byte();
                let end = node
                    .child_by_field_name(spec.return_type_field.unwrap_or("parameters"))
                    .map(|n| n.end_byte())
                    .or_else(|| {
                        node.child_by_field_name(spec.parameters_field)
                            .map(|n| n.end_byte())
                    })
                    .unwrap_or(node.end_byte());

                let body_start = node.child_by_field_name("body").map(|n| n.start_byte());
                let actual_end = body_start.map_or(end, |bs| bs.min(end + 200)).max(end);

                source[start..actual_end.min(source.len())]
                    .trim()
                    .to_string()
            }
            SignatureStyle::TypeKeyword(keyword) => {
                format!("{} {}", keyword, name)
            }
        },
        _ => {
            // For types/classes/structs/enums/interfaces/traits, find the right keyword
            let keyword = match kind {
                SymbolKind::Class => "class",
                SymbolKind::Struct => "struct",
                SymbolKind::Interface => "interface",
                SymbolKind::Enum => "enum",
                SymbolKind::Trait => "trait",
                SymbolKind::Type => "type",
                SymbolKind::Constant => "const",
                _ => "",
            };
            if keyword.is_empty() {
                name.to_string()
            } else {
                format!("{} {}", keyword, name)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::languages;

    // --- Python via generic extractor ---

    #[test]
    fn test_python_function_with_type_hints() {
        let spec = languages::python();
        let source = r#"def process_payment(amount: float, currency: str) -> Receipt:
    pass
"#;
        let symbols = extract(&spec, source, "pay.py");
        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "process_payment");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.return_type.as_deref(), Some("Receipt"));
        let params = sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "amount");
        assert_eq!(params[0].type_annotation.as_deref(), Some("float"));
        assert_eq!(params[1].name, "currency");
        assert_eq!(params[1].type_annotation.as_deref(), Some("str"));
    }

    #[test]
    fn test_python_class_with_methods() {
        let spec = languages::python();
        let source = r#"class AuthService:
    def __init__(self, db: Database):
        self.db = db

    def validate(self, token: str) -> bool:
        return True

    def _internal(self):
        pass
"#;
        let symbols = extract(&spec, source, "auth.py");
        // class + __init__ + validate (skip _internal)
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "AuthService");
        assert_eq!(symbols[0].kind, SymbolKind::Class);

        assert_eq!(symbols[1].name, "__init__");
        assert_eq!(symbols[1].kind, SymbolKind::Method);
        assert_eq!(symbols[1].parent_symbol.as_deref(), Some("AuthService"));
        // self should be filtered out
        let init_params = symbols[1].parameters.as_ref().unwrap();
        assert_eq!(init_params.len(), 1);
        assert_eq!(init_params[0].name, "db");

        assert_eq!(symbols[2].name, "validate");
        assert_eq!(symbols[2].kind, SymbolKind::Method);

        assert!(!symbols.iter().any(|s| s.name == "_internal"));
    }

    #[test]
    fn test_python_function_no_hints() {
        let spec = languages::python();
        let source = r#"def greet(name):
    return f"Hello {name}"
"#;
        let symbols = extract(&spec, source, "greet.py");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert!(symbols[0].return_type.is_none());
    }

    // --- Go via generic extractor ---

    #[test]
    fn test_go_exported_function() {
        let spec = languages::go();
        let source = r#"package main

func ProcessPayment(amount float64, currency string) error {
    return nil
}

func internalHelper() {}
"#;
        let symbols = extract(&spec, source, "pay.go");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "ProcessPayment");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert!(symbols[0].signature.as_ref().unwrap().contains("ProcessPayment"));
    }

    #[test]
    fn test_go_method() {
        let spec = languages::go();
        let source = r#"package main

func (s *Service) HandleRequest(req Request) Response {
    return Response{}
}
"#;
        let symbols = extract(&spec, source, "handler.go");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "HandleRequest");
        assert_eq!(symbols[0].kind, SymbolKind::Method);
    }

    // --- Rust via generic extractor ---

    #[test]
    fn test_rust_pub_function() {
        let spec = languages::rust();
        let source = r#"pub fn process(amount: f64) -> Result<()> {
    todo!()
}

fn internal() {}
"#;
        let symbols = extract(&spec, source, "lib.rs");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "process");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_rust_pub_struct_and_enum() {
        let spec = languages::rust();
        let source = r#"pub struct Config {
    pub name: String,
}

pub enum Status {
    Active,
    Inactive,
}

struct Internal {}
"#;
        let symbols = extract(&spec, source, "types.rs");
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Config");
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
        assert_eq!(symbols[1].name, "Status");
        assert_eq!(symbols[1].kind, SymbolKind::Enum);
    }
}
