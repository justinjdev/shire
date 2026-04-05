use super::{node_text, LanguageHooks, SymbolKind};
use tree_sitter::Node;

/// Build signature for Bash function definitions.
/// Extracts the function header up to (but not including) the body.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => {
            let start = node.start_byte();
            let end = node
                .child_by_field_name("body")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte());
            let sig = source[start..end.min(source.len())].trim();
            if sig.is_empty() {
                name.to_string()
            } else {
                sig.to_string()
            }
        }
        _ => node_text(node, source)
            .unwrap_or(name)
            .trim()
            .to_string(),
    }
}

/// Return Bash language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        build_signature: Some(build_signature),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::symbols::registry::extract_file;

    fn extract(source: &str) -> Vec<crate::symbols::SymbolInfo> {
        extract_file("sh", source, Arc::from("test.sh"), true).0
    }

    #[test]
    fn test_function_keyword_style() {
        let source = r#"function greet() {
    echo "Hello"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert_eq!(symbols[0].kind, crate::symbols::SymbolKind::Function);
        assert!(symbols[0].signature.as_ref().unwrap().contains("greet"));
    }

    #[test]
    fn test_function_shorthand_style() {
        let source = r#"greet() {
    echo "Hello"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert_eq!(symbols[0].kind, crate::symbols::SymbolKind::Function);
    }

    #[test]
    fn test_function_keyword_no_parens() {
        let source = r#"function cleanup {
    rm -rf /tmp/test
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "cleanup");
    }

    #[test]
    fn test_multiple_functions() {
        let source = r#"function setup() {
    echo "setup"
}

teardown() {
    echo "teardown"
}

function run {
    echo "run"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 3);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_ref()).collect();
        assert!(names.contains(&"setup"));
        assert!(names.contains(&"teardown"));
        assert!(names.contains(&"run"));
    }

    #[test]
    fn test_signature_includes_function_keyword() {
        let source = r#"function deploy() {
    echo "deploying"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("function deploy"));
    }

    #[test]
    fn test_no_parameters_or_return_type() {
        let source = r#"function foo() {
    return 0
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].parameters.as_ref().is_none_or(|p| p.is_empty()));
        assert!(symbols[0].return_type.is_none());
    }

    #[test]
    fn test_bash_extension() {
        let source = r#"function hello() {
    echo "world"
}"#;
        let symbols = extract_file("bash", source, Arc::from("test.bash"), true).0;
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
    }
}
