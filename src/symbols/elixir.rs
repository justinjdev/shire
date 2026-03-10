use super::{Parameter, SymbolInfo, SymbolKind};
use regex::Regex;
use std::sync::LazyLock;

static MODULE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*defmodule\s+([A-Z][\w.]*)\s+do\b").unwrap());
static PROTOCOL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*defprotocol\s+([A-Z][\w.]*)\s+do\b").unwrap());
static DEF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*def\s+(\w+)(\(([^)]*)\))?").unwrap());
static DEFMACRO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*defmacro\s+(\w+)(\(([^)]*)\))?").unwrap());
static CALLBACK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*@callback\s+(\w+)\((.*)\)\s*::\s*(.+)").unwrap());
static TYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*@type\s+(\w+)(?:\(([^)]*)\))?\s*::\s*(.+)").unwrap());
static END_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*end\b").unwrap());

/// Extract symbols from Elixir source code using regex-based parsing.
///
/// Elixir's tree-sitter grammar uses generic `call` nodes for everything
/// (def, defmodule, defprotocol, etc.), making tree-sitter queries impractical.
///
/// Extracts: modules (defmodule), public functions (def), public macros (defmacro),
/// protocols (defprotocol), callbacks (@callback), and type definitions (@type).
/// Skips private functions (defp), private macros (defmacrop), and private types (@typep).
pub fn extract(source: &str, file_path: &str) -> Vec<SymbolInfo> {
    let module_re = &*MODULE_RE;
    let protocol_re = &*PROTOCOL_RE;
    let def_re = &*DEF_RE;
    let defmacro_re = &*DEFMACRO_RE;
    let callback_re = &*CALLBACK_RE;
    let type_re = &*TYPE_RE;
    let end_re = &*END_RE;

    let mut symbols = Vec::new();
    // Stack of module names for tracking nesting
    let mut module_stack: Vec<String> = Vec::new();
    let mut depth: usize = 0;
    // Track depth at which each module was pushed
    let mut module_depths: Vec<usize> = Vec::new();

    for (line_idx, line) in source.lines().enumerate() {
        let line_number = line_idx + 1;
        let trimmed = line.trim();

        // Skip blanks and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for protocol definition
        if let Some(caps) = protocol_re.captures(line) {
            let name = caps[1].to_string();
            let signature = format!("defprotocol {} do", name);

            symbols.push(SymbolInfo {
                name: name.clone(),
                kind: SymbolKind::Interface,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: module_stack.last().cloned(),
                return_type: None,
                parameters: None,
            });

            depth += 1;
            module_depths.push(depth);
            module_stack.push(name);
            continue;
        }

        // Check for module definition
        if let Some(caps) = module_re.captures(line) {
            let name = caps[1].to_string();
            let signature = format!("defmodule {} do", name);

            symbols.push(SymbolInfo {
                name: name.clone(),
                kind: SymbolKind::Class,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: module_stack.last().cloned(),
                return_type: None,
                parameters: None,
            });

            depth += 1;
            module_depths.push(depth);
            module_stack.push(name);
            continue;
        }

        // Check for callback
        if let Some(caps) = callback_re.captures(line) {
            let name = caps[1].to_string();
            let params_str = caps[2].to_string();
            let return_type = caps[3].trim().to_string();
            let signature = format!("@callback {}({}) :: {}", name, params_str, return_type);

            let parameters = parse_type_params(&params_str);

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Method,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: module_stack.last().cloned(),
                return_type: Some(return_type),
                parameters: Some(parameters),
            });
            continue;
        }

        // Check for type definition
        if let Some(caps) = type_re.captures(line) {
            let name = caps[1].to_string();
            let params_str = caps.get(2).map(|m| m.as_str().to_string());
            let definition = caps[3].trim().to_string();

            let signature = match &params_str {
                Some(p) => format!("@type {}({}) :: {}", name, p, definition),
                None => format!("@type {} :: {}", name, definition),
            };

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Type,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: module_stack.last().cloned(),
                return_type: Some(definition),
                parameters: None,
            });
            continue;
        }

        // Check for public macro definition
        if let Some(caps) = defmacro_re.captures(line) {
            let name = caps[1].to_string();
            let params_str = caps.get(3).map(|m| m.as_str());
            let parameters = params_str.map(|p| parse_parameters(p));

            let signature = match params_str {
                Some(p) => format!("defmacro {}({})", name, p),
                None => format!("defmacro {}", name),
            };

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Function,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: module_stack.last().cloned(),
                return_type: None,
                parameters: Some(parameters.unwrap_or_default()),
            });

            // Only bump depth if this opens a do..end block
            if has_do_block(trimmed) {
                depth += 1;
            }
            continue;
        }

        // Check for public function definition
        if let Some(caps) = def_re.captures(line) {
            let name = caps[1].to_string();
            let params_str = caps.get(3).map(|m| m.as_str());
            let parameters = params_str.map(|p| parse_parameters(p));

            let signature = match params_str {
                Some(p) => format!("def {}({})", name, p),
                None => format!("def {}", name),
            };

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Function,
                signature: Some(signature),
                file_path: file_path.to_string(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: module_stack.last().cloned(),
                return_type: None,
                parameters: Some(parameters.unwrap_or_default()),
            });

            // Only bump depth if this opens a do..end block
            if has_do_block(trimmed) {
                depth += 1;
            }
            continue;
        }

        // Check for `end` keyword (pops context if needed)
        if end_re.is_match(line) {
            if depth > 0 {
                if let Some(&ctx_depth) = module_depths.last() {
                    if depth == ctx_depth {
                        module_stack.pop();
                        module_depths.pop();
                    }
                }
                depth -= 1;
            }
        }
    }

    symbols
}

/// Check if a line opens a do..end block (has `do` at end, not `do:` inline).
fn has_do_block(trimmed: &str) -> bool {
    // Lines ending with `do` open a block. Lines with `do:` are single-line.
    trimmed.ends_with(" do") || trimmed.ends_with("\tdo") || trimmed == "do"
}

/// Parse an Elixir parameter string like "name, age \\ 0, opts"
/// into a list of Parameter structs.
fn parse_parameters(params_str: &str) -> Vec<Parameter> {
    let mut params = Vec::new();

    for part in params_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Strip default values: "age \\ 0" -> "age"
        let name_part = if let Some(idx) = part.find("\\\\") {
            part[..idx].trim()
        } else {
            part
        };

        // Strip pattern match prefixes and destructuring
        // e.g., "%{name: name}" -> skip complex patterns
        // e.g., "_ignored" -> skip
        let clean = name_part.trim();

        // Skip complex patterns (maps, tuples, lists)
        if clean.starts_with('%')
            || clean.starts_with('{')
            || clean.starts_with('[')
            || clean.starts_with("<<")
        {
            continue;
        }

        // Strip leading underscore for unused vars but keep the name
        let clean = clean.trim_start_matches('_');

        // Handle pinned variables: ^name -> name
        let clean = clean.trim_start_matches('^');

        if !clean.is_empty() && clean.chars().next().unwrap().is_alphabetic() {
            // Take only the variable name (first word)
            let name = clean.split_whitespace().next().unwrap_or(clean);
            // Strip any trailing type-like suffixes from when clauses
            let name = name.split('=').next().unwrap_or(name).trim();

            if !name.is_empty() {
                params.push(Parameter {
                    name: name.to_string(),
                    type_annotation: None,
                });
            }
        }
    }

    params
}

/// Parse callback/spec type parameters like "integer(), String.t()" into Parameters
/// with type annotations.
fn parse_type_params(params_str: &str) -> Vec<Parameter> {
    let mut params = Vec::new();
    let mut paren_depth = 0;
    let mut current = String::new();

    for ch in params_str.chars() {
        match ch {
            '(' => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' => {
                paren_depth -= 1;
                current.push(ch);
            }
            ',' if paren_depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    params.push(Parameter {
                        name: format!("arg{}", params.len()),
                        type_annotation: Some(trimmed),
                    });
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        params.push(Parameter {
            name: format!("arg{}", params.len()),
            type_annotation: Some(trimmed),
        });
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_module() {
        let source = r#"
defmodule MyApp.Users do
  def list_users do
    []
  end
end
"#;
        let symbols = extract(source, "lib/my_app/users.ex");

        let mod_sym = symbols.iter().find(|s| s.name == "MyApp.Users").unwrap();
        assert_eq!(mod_sym.kind, SymbolKind::Class);
        assert_eq!(
            mod_sym.signature.as_deref(),
            Some("defmodule MyApp.Users do")
        );
        assert_eq!(mod_sym.line, 2);
        assert!(mod_sym.parent_symbol.is_none());
    }

    #[test]
    fn test_extract_public_function() {
        let source = r#"
defmodule Calculator do
  def add(a, b) do
    a + b
  end
end
"#;
        let symbols = extract(source, "lib/calculator.ex");

        let func = symbols.iter().find(|s| s.name == "add").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.signature.as_deref(), Some("def add(a, b)"));
        assert_eq!(func.parent_symbol.as_deref(), Some("Calculator"));
        assert_eq!(func.line, 3);

        let params = func.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "a");
        assert_eq!(params[1].name, "b");
    }

    #[test]
    fn test_skip_private_functions() {
        let source = r#"
defmodule MyModule do
  def public_fn do
    :ok
  end

  defp private_fn do
    :secret
  end
end
"#;
        let symbols = extract(source, "lib/my_module.ex");

        assert!(symbols.iter().any(|s| s.name == "public_fn"));
        assert!(!symbols.iter().any(|s| s.name == "private_fn"));
    }

    #[test]
    fn test_extract_protocol() {
        let source = r#"
defprotocol MyApp.Serializable do
  @doc "Serializes the given value"
  def serialize(value)
end
"#;
        let symbols = extract(source, "lib/my_app/serializable.ex");

        let proto = symbols
            .iter()
            .find(|s| s.name == "MyApp.Serializable")
            .unwrap();
        assert_eq!(proto.kind, SymbolKind::Interface);
        assert_eq!(
            proto.signature.as_deref(),
            Some("defprotocol MyApp.Serializable do")
        );

        let func = symbols.iter().find(|s| s.name == "serialize").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.parent_symbol.as_deref(), Some("MyApp.Serializable"));
    }

    #[test]
    fn test_extract_macro() {
        let source = r#"
defmodule MyApp.Router do
  defmacro route(path, handler) do
    quote do
      @routes [{unquote(path), unquote(handler)} | @routes]
    end
  end
end
"#;
        let symbols = extract(source, "lib/my_app/router.ex");

        let mac = symbols.iter().find(|s| s.name == "route").unwrap();
        assert_eq!(mac.kind, SymbolKind::Function);
        assert_eq!(
            mac.signature.as_deref(),
            Some("defmacro route(path, handler)")
        );
        assert_eq!(mac.parent_symbol.as_deref(), Some("MyApp.Router"));

        let params = mac.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "path");
        assert_eq!(params[1].name, "handler");
    }

    #[test]
    fn test_extract_callback() {
        let source = r#"
defmodule MyApp.Behaviour do
  @callback init(opts :: keyword()) :: {:ok, state :: term()} | {:error, reason :: term()}
end
"#;
        let symbols = extract(source, "lib/my_app/behaviour.ex");

        let cb = symbols.iter().find(|s| s.name == "init").unwrap();
        assert_eq!(cb.kind, SymbolKind::Method);
        assert!(cb
            .signature
            .as_deref()
            .unwrap()
            .starts_with("@callback init("));
        assert_eq!(cb.parent_symbol.as_deref(), Some("MyApp.Behaviour"));
        assert!(cb.return_type.is_some());
    }

    #[test]
    fn test_extract_type() {
        let source = r#"
defmodule MyApp.Types do
  @type name :: String.t()
  @type pair(a, b) :: {a, b}
end
"#;
        let symbols = extract(source, "lib/my_app/types.ex");

        let name_type = symbols.iter().find(|s| s.name == "name").unwrap();
        assert_eq!(name_type.kind, SymbolKind::Type);
        assert_eq!(
            name_type.signature.as_deref(),
            Some("@type name :: String.t()")
        );
        assert_eq!(name_type.return_type.as_deref(), Some("String.t()"));

        let pair_type = symbols.iter().find(|s| s.name == "pair").unwrap();
        assert_eq!(pair_type.kind, SymbolKind::Type);
    }

    #[test]
    fn test_nested_modules() {
        let source = r#"
defmodule MyApp.Outer do
  defmodule Inner do
    def hello do
      :world
    end
  end
end
"#;
        let symbols = extract(source, "lib/my_app/outer.ex");

        let outer = symbols.iter().find(|s| s.name == "MyApp.Outer").unwrap();
        assert!(outer.parent_symbol.is_none());

        let inner = symbols.iter().find(|s| s.name == "Inner").unwrap();
        assert_eq!(inner.parent_symbol.as_deref(), Some("MyApp.Outer"));

        let func = symbols.iter().find(|s| s.name == "hello").unwrap();
        assert_eq!(func.parent_symbol.as_deref(), Some("Inner"));
    }

    #[test]
    fn test_function_with_defaults() {
        let source = r#"
defmodule Config do
  def load(path, opts \\ []) do
    :ok
  end
end
"#;
        let symbols = extract(source, "lib/config.ex");

        let func = symbols.iter().find(|s| s.name == "load").unwrap();
        let params = func.parameters.as_ref().unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "path");
        assert_eq!(params[1].name, "opts");
    }

    #[test]
    fn test_function_with_guard() {
        let source = r#"
defmodule Math do
  def abs(x) when x >= 0 do
    x
  end
end
"#;
        let symbols = extract(source, "lib/math.ex");

        let func = symbols.iter().find(|s| s.name == "abs").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.signature.as_deref(), Some("def abs(x)"));
    }

    #[test]
    fn test_one_line_function() {
        let source = r#"
defmodule MyModule do
  def greet(name), do: "Hello, #{name}"
end
"#;
        let symbols = extract(source, "lib/my_module.ex");

        let func = symbols.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.parent_symbol.as_deref(), Some("MyModule"));
    }

    #[test]
    fn test_empty_source() {
        let symbols = extract("", "empty.ex");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_line_numbers() {
        let source = r#"defmodule Foo do
  def bar do
    :ok
  end

  def baz(x) do
    x
  end
end
"#;
        let symbols = extract(source, "lib/foo.ex");

        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].line, 1); // defmodule Foo
        assert_eq!(symbols[1].line, 2); // def bar
        assert_eq!(symbols[2].line, 6); // def baz
    }

    #[test]
    fn test_comments_ignored() {
        let source = r#"
defmodule Foo do
  # def not_a_function do
  #   :nope
  # end

  def real_function do
    :ok
  end
end
"#;
        let symbols = extract(source, "lib/foo.ex");

        assert_eq!(symbols.len(), 2); // module + real_function
        assert!(!symbols.iter().any(|s| s.name == "not_a_function"));
    }

    #[test]
    fn test_dotted_module_name() {
        let source = "defmodule MyApp.Web.Controllers.UserController do\nend\n";
        let symbols = extract(source, "lib/my_app/web/controllers/user_controller.ex");

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "MyApp.Web.Controllers.UserController");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
    }
}
