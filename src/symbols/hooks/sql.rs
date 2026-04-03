use super::{LanguageHooks, SymbolKind};
use tree_sitter::Node;

/// Build signature for SQL DDL definitions.
/// Extracts the statement header up to (but not including) the body.
fn build_signature(node: &Node, source: &str, name: &str, kind: SymbolKind) -> String {
    let start = node.start_byte();

    // Find the end of the signature: look for body-like children
    let end = find_body_start(node).unwrap_or(node.end_byte());

    let sig = source[start..end.min(source.len())].trim();
    if sig.is_empty() {
        return name.to_string();
    }

    // Trim trailing semicolons and whitespace
    let sig = sig.trim_end_matches(';').trim();

    // For single-line signatures, return as-is; for multi-line, take the first line
    match kind {
        SymbolKind::Class => {
            // Tables: take up to (but not including) column definitions
            sig.to_string()
        }
        _ => sig.to_string(),
    }
}

/// Find the byte offset where the "body" of a DDL statement begins.
/// This is used to truncate signatures before the body content.
fn find_body_start(node: &Node) -> Option<usize> {
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        match child.kind() {
            // Table column definitions
            "column_definitions" => return Some(child.start_byte()),
            // Function/procedure body
            "function_body" | "procedure_body" => return Some(child.start_byte()),
            // View query (the AS ... SELECT part)
            "keyword_as" => return Some(child.start_byte()),
            // Index fields
            "index_fields" => return Some(child.start_byte()),
            // Enum elements for CREATE TYPE
            "enum_elements" => return Some(child.start_byte()),
            // Trigger execution clause
            "keyword_execute" => return Some(child.start_byte()),
            _ => {}
        }
    }
    None
}

/// Return SQL language hooks.
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
    use crate::symbols::SymbolKind;

    fn extract(source: &str) -> Vec<crate::symbols::SymbolInfo> {
        extract_file("sql", source, Arc::from("test.sql"))
    }

    #[test]
    fn test_create_table() {
        let source = r#"CREATE TABLE users (
    id INT PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) UNIQUE
);"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "users");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.contains("CREATE TABLE"));
        assert!(sig.contains("users"));
        // Signature should not include column definitions
        assert!(!sig.contains("id INT"));
    }

    #[test]
    fn test_create_view() {
        let source = r#"CREATE OR REPLACE VIEW active_users AS
SELECT * FROM users WHERE active = true;"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "active_users");
        assert_eq!(symbols[0].kind, SymbolKind::Interface);
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.contains("VIEW"));
        assert!(sig.contains("active_users"));
    }

    #[test]
    fn test_create_function() {
        let source = r#"CREATE FUNCTION get_user(user_id INT)
RETURNS INT
AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql;"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "get_user");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_create_trigger() {
        let source = r#"CREATE TRIGGER audit_trigger
AFTER INSERT ON users
FOR EACH ROW
EXECUTE FUNCTION audit_log();"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "audit_trigger");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_create_type() {
        let source = r#"CREATE TYPE status_type AS ENUM ('active', 'inactive', 'pending');"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "status_type");
        assert_eq!(symbols[0].kind, SymbolKind::Type);
    }

    #[test]
    fn test_create_index() {
        let source = r#"CREATE INDEX idx_users_email ON users (email);"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "idx_users_email");
        assert_eq!(symbols[0].kind, SymbolKind::Constant);
    }

    #[test]
    fn test_multiple_statements() {
        let source = r#"CREATE TABLE orders (id INT PRIMARY KEY, total DECIMAL);

CREATE VIEW order_summary AS
SELECT * FROM orders WHERE total > 100;

CREATE FUNCTION calc_tax(amount DECIMAL)
RETURNS DECIMAL
AS $$ BEGIN RETURN amount * 0.1; END; $$ LANGUAGE plpgsql;

CREATE TYPE priority AS ENUM ('low', 'medium', 'high');"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 4);

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_ref()).collect();
        assert!(names.contains(&"orders"));
        assert!(names.contains(&"order_summary"));
        assert!(names.contains(&"calc_tax"));
        assert!(names.contains(&"priority"));
    }

    #[test]
    fn test_table_signature_excludes_body() {
        let source = r#"CREATE TABLE products (
    id INT PRIMARY KEY,
    name VARCHAR(100)
);"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("CREATE TABLE"));
        assert!(!sig.contains("INT PRIMARY KEY"));
    }
}
