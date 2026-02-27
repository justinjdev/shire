use super::{Parameter, SymbolInfo, SymbolKind};
use regex::Regex;

/// Extract symbols from Ruby source code using regex-based parsing.
///
/// Extracts classes (with inheritance), modules, instance methods, class methods,
/// and top-level functions. Uses recursive-style line scanning to track
/// parent class/module context.
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let class_re = Regex::new(r"^\s*class\s+(\w+)(?:\s*<\s*(\w+))?").unwrap();
    let module_re = Regex::new(r"^\s*module\s+(\w+)").unwrap();
    let method_re = Regex::new(r"^\s*def\s+(self\.)?(\w+[!?=]?)(?:\s*\(([^)]*)\))?").unwrap();
    let end_re = Regex::new(r"^\s*end\b").unwrap();

    let mut symbols = Vec::new();
    // Stack of (name, kind, indent_depth) for tracking nesting
    let mut context_stack: Vec<(String, &str)> = Vec::new();
    let mut depth: usize = 0;
    // Track depth at which each context was pushed
    let mut context_depths: Vec<usize> = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        let line_number = line_idx + 1;
        let trimmed = line.trim();

        // Skip blanks and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for class definition
        if let Some(caps) = class_re.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            let superclass = caps.get(2).map(|m| m.as_str().to_string());

            let signature = match &superclass {
                Some(parent) => format!("class {} < {}", name, parent),
                None => format!("class {}", name),
            };

            symbols.push(SymbolInfo {
                name: name.clone(),
                kind: SymbolKind::Class,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: context_stack.last().map(|(n, _)| n.clone()),
                return_type: None,
                parameters: None,
            });

            depth += 1;
            context_depths.push(depth);
            context_stack.push((name, "class"));
            continue;
        }

        // Check for module definition
        if let Some(caps) = module_re.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();

            symbols.push(SymbolInfo {
                name: name.clone(),
                kind: SymbolKind::Class,
                signature: Some(format!("module {}", name)),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: context_stack.last().map(|(n, _)| n.clone()),
                return_type: None,
                parameters: None,
            });

            depth += 1;
            context_depths.push(depth);
            context_stack.push((name, "module"));
            continue;
        }

        // Check for method definition
        if let Some(caps) = method_re.captures(line) {
            let is_class_method = caps.get(1).is_some();
            let method_name = caps.get(2).unwrap().as_str().to_string();
            let params_str = caps.get(3).map(|m| m.as_str());

            let parameters = params_str.map(|p| parse_parameters(p));

            let parent_symbol = context_stack.last().map(|(n, _)| n.clone());

            let (kind, signature) = if is_class_method {
                let parent_prefix = parent_symbol
                    .as_deref()
                    .map(|p| format!("{}.", p))
                    .unwrap_or_default();
                let sig = match params_str {
                    Some(p) => format!("def self.{}({})", method_name, p),
                    None => format!("def self.{}", method_name),
                };
                let _ = parent_prefix;
                (SymbolKind::Function, sig)
            } else if parent_symbol.is_some() {
                let sig = match params_str {
                    Some(p) => format!("def {}({})", method_name, p),
                    None => format!("def {}", method_name),
                };
                (SymbolKind::Method, sig)
            } else {
                let sig = match params_str {
                    Some(p) => format!("def {}({})", method_name, p),
                    None => format!("def {}", method_name),
                };
                (SymbolKind::Function, sig)
            };

            symbols.push(SymbolInfo {
                name: method_name,
                kind,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol,
                return_type: None,
                parameters: Some(parameters.unwrap_or_default()),
            });

            depth += 1;
            continue;
        }

        // Check for `end` keyword (pops context if needed)
        if end_re.is_match(line) {
            if depth > 0 {
                // Check if this `end` closes a class/module context
                if let Some(&ctx_depth) = context_depths.last() {
                    if depth == ctx_depth {
                        context_stack.pop();
                        context_depths.pop();
                    }
                }
                depth -= 1;
            }
        }
    }

    symbols
}

/// Parse a Ruby parameter string like "name, age = 0, *args, **opts"
/// into a list of Parameter structs.
fn parse_parameters(params_str: &str) -> Vec<Parameter> {
    let mut params = Vec::new();

    for part in params_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Strip default values: "age = 0" -> "age"
        let name_part = if let Some(idx) = part.find('=') {
            part[..idx].trim()
        } else {
            part
        };

        // Strip splat/double-splat/block prefixes but keep them for type info
        let (clean_name, type_hint) = if name_part.starts_with("**") {
            (&name_part[2..], Some("**".to_string()))
        } else if name_part.starts_with('*') {
            (&name_part[1..], Some("*".to_string()))
        } else if name_part.starts_with('&') {
            (&name_part[1..], Some("&".to_string()))
        } else {
            (name_part, None)
        };

        // Handle "name:" keyword args
        let clean_name = clean_name.trim_end_matches(':');

        if !clean_name.is_empty() {
            params.push(Parameter {
                name: clean_name.to_string(),
                type_annotation: type_hint,
            });
        }
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_class() {
        let source = r#"
class UserService
  def initialize(db)
    @db = db
  end
end
"#;
        let symbols = extract(source, "app/services/user_service.rb");

        let class_sym = symbols.iter().find(|s| s.name == "UserService").unwrap();
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.signature.as_deref(), Some("class UserService"));
        assert_eq!(class_sym.line, 2);
    }

    #[test]
    fn test_extract_module() {
        let source = r#"
module Authentication
  def authenticate(token)
    verify(token)
  end
end
"#;
        let symbols = extract(source, "app/concerns/authentication.rb");

        let mod_sym = symbols.iter().find(|s| s.name == "Authentication").unwrap();
        assert_eq!(mod_sym.kind, SymbolKind::Class);
        assert_eq!(mod_sym.signature.as_deref(), Some("module Authentication"));
    }

    #[test]
    fn test_extract_instance_method() {
        let source = r#"
class OrderProcessor
  def process(order_id, amount)
    # process logic
  end
end
"#;
        let symbols = extract(source, "app/services/order_processor.rb");

        let method_sym = symbols.iter().find(|s| s.name == "process").unwrap();
        assert_eq!(method_sym.kind, SymbolKind::Method);
        assert_eq!(method_sym.parent_symbol.as_deref(), Some("OrderProcessor"));
        assert_eq!(
            method_sym.signature.as_deref(),
            Some("def process(order_id, amount)")
        );

        let params = method_sym.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "order_id");
        assert_eq!(params[1].name, "amount");
    }

    #[test]
    fn test_extract_class_method() {
        let source = r#"
class Config
  def self.load(path)
    new(path)
  end
end
"#;
        let symbols = extract(source, "lib/config.rb");

        let method_sym = symbols.iter().find(|s| s.name == "load").unwrap();
        assert_eq!(method_sym.kind, SymbolKind::Function);
        assert_eq!(method_sym.parent_symbol.as_deref(), Some("Config"));
        assert_eq!(method_sym.signature.as_deref(), Some("def self.load(path)"));
    }

    #[test]
    fn test_extract_top_level_function() {
        let source = r#"
def main
  puts "hello"
end
"#;
        let symbols = extract(source, "script.rb");

        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "main");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.parent_symbol.is_none());
        assert_eq!(sym.signature.as_deref(), Some("def main"));
    }

    #[test]
    fn test_extract_inheritance() {
        let source = r#"
class AdminController < ApplicationController
  def index
    render :index
  end
end
"#;
        let symbols = extract(source, "app/controllers/admin_controller.rb");

        let class_sym = symbols
            .iter()
            .find(|s| s.name == "AdminController")
            .unwrap();
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(
            class_sym.signature.as_deref(),
            Some("class AdminController < ApplicationController")
        );

        let method_sym = symbols.iter().find(|s| s.name == "index").unwrap();
        assert_eq!(method_sym.kind, SymbolKind::Method);
        assert_eq!(method_sym.parent_symbol.as_deref(), Some("AdminController"));
    }

    #[test]
    fn test_extract_nested_class_in_module() {
        let source = r#"
module Payments
  class Processor
    def charge(amount)
      # charge logic
    end
  end
end
"#;
        let symbols = extract(source, "lib/payments/processor.rb");

        let mod_sym = symbols.iter().find(|s| s.name == "Payments").unwrap();
        assert_eq!(mod_sym.kind, SymbolKind::Class);

        let class_sym = symbols.iter().find(|s| s.name == "Processor").unwrap();
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.parent_symbol.as_deref(), Some("Payments"));

        let method_sym = symbols.iter().find(|s| s.name == "charge").unwrap();
        assert_eq!(method_sym.kind, SymbolKind::Method);
        assert_eq!(method_sym.parent_symbol.as_deref(), Some("Processor"));
    }

    #[test]
    fn test_extract_method_with_special_params() {
        let source = r#"
def create(name, *args, **opts, &block)
  # ...
end
"#;
        let symbols = extract(source, "factory.rb");

        assert_eq!(symbols.len(), 1);
        let params = symbols[0].parameters.as_ref().unwrap();
        assert_eq!(params.len(), 4);
        assert_eq!(params[0].name, "name");
        assert!(params[0].type_annotation.is_none());
        assert_eq!(params[1].name, "args");
        assert_eq!(params[1].type_annotation.as_deref(), Some("*"));
        assert_eq!(params[2].name, "opts");
        assert_eq!(params[2].type_annotation.as_deref(), Some("**"));
        assert_eq!(params[3].name, "block");
        assert_eq!(params[3].type_annotation.as_deref(), Some("&"));
    }
}
