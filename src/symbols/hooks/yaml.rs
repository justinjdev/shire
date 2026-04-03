use super::{find_child_by_kind, node_text, LanguageHooks, SymbolKind};
use tree_sitter::Node;

/// Build signature for YAML top-level keys.
/// Shows the key name with a hint about the value type (mapping, sequence, or scalar).
fn build_signature(node: &Node, source: &str, name: &str, _kind: SymbolKind) -> String {
    if let Some(value_node) = node.child_by_field_name("value") {
        let value_kind = value_node.kind();
        // block_node wraps nested mappings and sequences
        if value_kind == "block_node" {
            if let Some(inner) = find_child_by_kind(&value_node, "block_mapping") {
                let count = inner.named_child_count();
                return format!("{}: {{...}} ({} keys)", name, count);
            }
            if let Some(inner) = find_child_by_kind(&value_node, "block_sequence") {
                let count = inner.named_child_count();
                return format!("{}: [...] ({} items)", name, count);
            }
        }
        // flow_node wraps inline scalars
        if let Some(text) = node_text(&value_node, source) {
            let text = text.trim();
            if text.len() <= 60 {
                return format!("{}: {}", name, text);
            }
        }
    }
    name.to_string()
}

/// Return YAML language hooks.
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
        extract_file("yaml", source, Arc::from("test.yaml"))
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
        let symbols = extract_file("yml", source, Arc::from("test.yml"));
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "key");
    }
}
