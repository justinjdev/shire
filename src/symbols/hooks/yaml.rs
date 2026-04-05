use super::{find_child_by_kind, node_text, LanguageHooks, SymbolInfo, SymbolKind};
use tree_sitter::Node;

/// Strip surrounding quotes from a YAML key name.
fn strip_quotes(name: &str) -> String {
    if let Some(stripped) = name.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return stripped.to_string();
    }
    if let Some(stripped) = name.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return stripped.to_string();
    }
    name.to_string()
}

/// Build signature for YAML top-level keys.
/// Shows the key name with a hint about the value type (mapping, sequence, or scalar).
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    let clean_name = strip_quotes(name);
    if let Some(value_node) = node.child_by_field_name("value") {
        let value_kind = value_node.kind();
        // block_node wraps nested mappings and sequences
        if value_kind == "block_node" {
            if let Some(inner) = find_child_by_kind(&value_node, "block_mapping") {
                let count = inner.named_child_count();
                return format!("{}: {{...}} ({} keys)", clean_name, count);
            }
            if let Some(inner) = find_child_by_kind(&value_node, "block_sequence") {
                let count = inner.named_child_count();
                return format!("{}: [...] ({} items)", clean_name, count);
            }
        }
        // flow_node wraps inline scalars
        if let Some(text) = node_text(&value_node, source) {
            let text = text.trim().replace('\n', " ");
            if text.len() <= 60 {
                return format!("{}: {}", clean_name, text);
            }
        }
    }
    clean_name
}

/// Post-process: strip quotes from key names captured by the double/single quote patterns.
fn post_process(mut sym: SymbolInfo, _node: &Node, _source: &str) -> Option<SymbolInfo> {
    sym.name = strip_quotes(&sym.name);
    Some(sym)
}

/// Return YAML language hooks.
pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        build_signature: Some(build_signature),
        post_process: Some(post_process),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::symbols::registry::extract_file;

    fn extract(source: &str) -> Vec<crate::symbols::SymbolInfo> {
        extract_file("yaml", source, Arc::from("test.yaml"), true).0
    }

    #[test]
    fn test_simple_key_value() {
        let source = "name: my-app\n";
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "name");
        assert_eq!(symbols[0].kind, crate::symbols::SymbolKind::Constant);
        assert!(symbols[0].signature.as_ref().unwrap().contains("my-app"));
    }

    #[test]
    fn test_multiple_top_level_keys() {
        let source = "name: app\nversion: \"1.0\"\nauthor: someone\n";
        let symbols = extract(source);
        assert_eq!(symbols.len(), 3);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_ref()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"version"));
        assert!(names.contains(&"author"));
    }

    #[test]
    fn test_nested_keys_not_extracted() {
        let source = "services:\n  web:\n    image: nginx\n    ports:\n      - \"80:80\"\n";
        let symbols = extract(source);
        // Only "services" should be extracted, not web/image/ports
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "services");
    }

    #[test]
    fn test_mapping_value_signature() {
        let source = "database:\n  host: localhost\n  port: 5432\n";
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.contains("{...}"), "signature should hint at mapping: {}", sig);
    }

    #[test]
    fn test_sequence_value_signature() {
        let source = "dependencies:\n  - lodash\n  - express\n";
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.contains("[...]"), "signature should hint at sequence: {}", sig);
    }

    #[test]
    fn test_yml_extension() {
        let source = "key: value\n";
        let symbols = extract_file("yml", source, Arc::from("test.yml"), true).0;
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "key");
    }

    #[test]
    fn test_double_quoted_key() {
        let source = "\"on\": push\n";
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "on");
    }

    #[test]
    fn test_single_quoted_key() {
        let source = "'version': \"2.0\"\n";
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "version");
    }

    #[test]
    fn test_mixed_quoted_and_plain_keys() {
        let source = "name: app\n\"on\":\n  push:\n    branches: [main]\n'jobs':\n  build:\n    runs-on: ubuntu-latest\n";
        let symbols = extract(source);
        assert_eq!(symbols.len(), 3);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_ref()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"on"));
        assert!(names.contains(&"jobs"));
    }

    #[test]
    fn test_empty_yaml() {
        let source = "";
        let symbols = extract(source);
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_comment_only_yaml() {
        let source = "# This is a comment\n# Another comment\n";
        let symbols = extract(source);
        assert!(symbols.is_empty());
    }
}
