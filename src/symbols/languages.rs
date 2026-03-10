use super::lang_spec::*;
use super::SymbolKind;

pub fn python() -> LanguageSpec {
    LanguageSpec {
        extensions: &["py"],
        ts_language: || tree_sitter_python::LANGUAGE.into(),
        definition_nodes: &[
            NodeMapping {
                node_type: "function_definition",
                symbol_kind: SymbolKind::Function,
            },
            NodeMapping {
                node_type: "class_definition",
                symbol_kind: SymbolKind::Class,
            },
        ],
        name_field: "name",
        parameters_field: "parameters",
        param_specs: &[
            ParamSpec {
                kind: "identifier",
                name_source: FieldSource::NodeText,
                type_source: None,
            },
            ParamSpec {
                kind: "typed_parameter",
                name_source: FieldSource::Child(0),
                type_source: Some(FieldSource::Field("type")),
            },
            ParamSpec {
                kind: "typed_default_parameter",
                name_source: FieldSource::Field("name"),
                type_source: Some(FieldSource::Field("type")),
            },
            ParamSpec {
                kind: "default_parameter",
                name_source: FieldSource::Field("name"),
                type_source: None,
            },
        ],
        return_type_field: Some("return_type"),
        return_type_unwrap_colon: false,
        visibility: VisibilityRule::AllPublic,
        fn_signature: SignatureStyle::KeywordBased("def"),
        method_spec: Some(MethodSpec {
            body_field: "body",
            method_node_kinds: &["function_definition"],
            self_param_names: &["self"],
            self_param_kinds: &[],
            visibility: Some(MethodVisibility::SkipPrefix("_")),
        }),
        constructor_name: Some("__init__"),
    }
}

pub fn go() -> LanguageSpec {
    LanguageSpec {
        extensions: &["go"],
        ts_language: || tree_sitter_go::LANGUAGE.into(),
        definition_nodes: &[
            NodeMapping {
                node_type: "function_declaration",
                symbol_kind: SymbolKind::Function,
            },
            NodeMapping {
                node_type: "method_declaration",
                symbol_kind: SymbolKind::Method,
            },
        ],
        name_field: "name",
        parameters_field: "parameters",
        param_specs: &[ParamSpec {
            kind: "parameter_declaration",
            name_source: FieldSource::Field("name"),
            type_source: Some(FieldSource::Field("type")),
        }],
        return_type_field: Some("result"),
        return_type_unwrap_colon: false,
        visibility: VisibilityRule::UppercaseExported,
        fn_signature: SignatureStyle::SourceSpan,
        method_spec: None, // Go doesn't have class bodies; methods are top-level
        constructor_name: None,
    }
}

pub fn rust() -> LanguageSpec {
    LanguageSpec {
        extensions: &["rs"],
        ts_language: || tree_sitter_rust::LANGUAGE.into(),
        definition_nodes: &[
            NodeMapping {
                node_type: "function_item",
                symbol_kind: SymbolKind::Function,
            },
            NodeMapping {
                node_type: "struct_item",
                symbol_kind: SymbolKind::Struct,
            },
            NodeMapping {
                node_type: "enum_item",
                symbol_kind: SymbolKind::Enum,
            },
            NodeMapping {
                node_type: "trait_item",
                symbol_kind: SymbolKind::Trait,
            },
        ],
        name_field: "name",
        parameters_field: "parameters",
        param_specs: &[ParamSpec {
            kind: "parameter",
            name_source: FieldSource::Field("pattern"),
            type_source: Some(FieldSource::Field("type")),
        }],
        return_type_field: Some("return_type"),
        return_type_unwrap_colon: false,
        visibility: VisibilityRule::HasChildNode("visibility_modifier"),
        fn_signature: SignatureStyle::SourceSpan,
        method_spec: None, // Rust impl blocks need custom handling
        constructor_name: None,
    }
}
