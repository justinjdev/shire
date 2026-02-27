use super::{SymbolInfo, SymbolKind};
use regex::Regex;

/// Extract symbols from Perl source code using regex-based parsing.
///
/// Extracts package declarations as Class symbols and sub definitions as
/// Function (top-level) or Method (inside a package block) symbols.
/// Subs starting with `_` are skipped (Perl private convention).
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let package_re = Regex::new(r"^\s*package\s+([\w:]+)").unwrap();
    let sub_re = Regex::new(r"^\s*sub\s+(\w+)").unwrap();

    let mut symbols = Vec::new();
    let mut current_package: Option<String> = None;

    for (line_idx, line) in source.lines().enumerate() {
        let line_number = line_idx + 1;

        if let Some(caps) = package_re.captures(line) {
            let name = caps[1].to_string();
            let signature = format!("package {}", name);

            symbols.push(SymbolInfo {
                name: name.clone(),
                kind: SymbolKind::Class,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: None,
                return_type: None,
                parameters: None,
            });

            current_package = Some(name);
            continue;
        }

        if let Some(caps) = sub_re.captures(line) {
            let name = caps[1].to_string();

            // Skip private subs (starting with _)
            if name.starts_with('_') {
                continue;
            }

            let (kind, parent_symbol, signature) = match &current_package {
                Some(pkg) => (
                    SymbolKind::Method,
                    Some(pkg.clone()),
                    format!("sub {}::{}", pkg, name),
                ),
                None => (
                    SymbolKind::Function,
                    None,
                    format!("sub {}", name),
                ),
            };

            symbols.push(SymbolInfo {
                name,
                kind,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol,
                return_type: None,
                parameters: None,
            });
        }
    }

    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_package_declaration() {
        let source = "package Foo::Bar;\n\nuse strict;\n";
        let symbols = extract(source, "lib/Foo/Bar.pm");

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Foo::Bar");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        assert_eq!(symbols[0].signature.as_deref(), Some("package Foo::Bar"));
        assert_eq!(symbols[0].line, 1);
        assert!(symbols[0].parent_symbol.is_none());
    }

    #[test]
    fn test_extract_top_level_sub() {
        let source = r#"
sub greet {
    my ($name) = @_;
    print "Hello, $name\n";
}
"#;
        let symbols = extract(source, "script.pl");

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].signature.as_deref(), Some("sub greet"));
        assert!(symbols[0].parent_symbol.is_none());
    }

    #[test]
    fn test_extract_method_inside_package() {
        let source = r#"package MyApp::Auth;

sub new {
    my ($class, %args) = @_;
    return bless \%args, $class;
}

sub validate {
    my ($self, $token) = @_;
    return 1;
}
"#;
        let symbols = extract(source, "lib/MyApp/Auth.pm");

        assert_eq!(symbols.len(), 3);

        let pkg = &symbols[0];
        assert_eq!(pkg.name, "MyApp::Auth");
        assert_eq!(pkg.kind, SymbolKind::Class);

        let new_sub = &symbols[1];
        assert_eq!(new_sub.name, "new");
        assert_eq!(new_sub.kind, SymbolKind::Method);
        assert_eq!(new_sub.parent_symbol.as_deref(), Some("MyApp::Auth"));
        assert_eq!(
            new_sub.signature.as_deref(),
            Some("sub MyApp::Auth::new")
        );

        let validate_sub = &symbols[2];
        assert_eq!(validate_sub.name, "validate");
        assert_eq!(validate_sub.kind, SymbolKind::Method);
        assert_eq!(validate_sub.parent_symbol.as_deref(), Some("MyApp::Auth"));
    }

    #[test]
    fn test_skip_private_subs() {
        let source = r#"package Foo;

sub public_method {
    return 1;
}

sub _private_helper {
    return 2;
}

sub _another_private {
    return 3;
}
"#;
        let symbols = extract(source, "lib/Foo.pm");

        assert_eq!(symbols.len(), 2); // package + public_method only
        assert_eq!(symbols[0].name, "Foo");
        assert_eq!(symbols[1].name, "public_method");
    }

    #[test]
    fn test_multiple_packages() {
        let source = r#"package First::Package;

sub alpha {
    return 1;
}

package Second::Package;

sub beta {
    return 2;
}
"#;
        let symbols = extract(source, "lib/Multi.pm");

        assert_eq!(symbols.len(), 4);

        assert_eq!(symbols[0].name, "First::Package");
        assert_eq!(symbols[0].kind, SymbolKind::Class);

        assert_eq!(symbols[1].name, "alpha");
        assert_eq!(symbols[1].kind, SymbolKind::Method);
        assert_eq!(symbols[1].parent_symbol.as_deref(), Some("First::Package"));

        assert_eq!(symbols[2].name, "Second::Package");
        assert_eq!(symbols[2].kind, SymbolKind::Class);

        assert_eq!(symbols[3].name, "beta");
        assert_eq!(symbols[3].kind, SymbolKind::Method);
        assert_eq!(
            symbols[3].parent_symbol.as_deref(),
            Some("Second::Package")
        );
    }

    #[test]
    fn test_empty_source() {
        let symbols = extract("", "empty.pl");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_comments_and_noise() {
        let source = r#"# This is a Perl module
use strict;
use warnings;

package Utils;

# A utility function
sub helper {
    return 42;
}

# This line has sub in a comment but should not match
# sub fake_sub { ... }

1;
"#;
        let symbols = extract(source, "lib/Utils.pm");

        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "Utils");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        assert_eq!(symbols[1].name, "helper");
        assert_eq!(symbols[1].kind, SymbolKind::Method);
    }

    #[test]
    fn test_line_numbers() {
        let source = r#"package Foo;

sub bar {
    return 1;
}

sub baz {
    return 2;
}
"#;
        let symbols = extract(source, "lib/Foo.pm");

        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].line, 1); // package Foo
        assert_eq!(symbols[1].line, 3); // sub bar
        assert_eq!(symbols[2].line, 7); // sub baz
    }
}
