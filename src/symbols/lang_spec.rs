use super::SymbolKind;

/// Maps a tree-sitter node type to a SymbolKind for extraction.
pub struct NodeMapping {
    pub node_type: &'static str,
    pub symbol_kind: SymbolKind,
}

/// Describes how to extract a parameter name and optional type from a parameter node.
pub struct ParamSpec {
    /// The tree-sitter node kind (e.g., "typed_parameter", "parameter_declaration")
    pub kind: &'static str,
    /// How to extract the parameter name
    pub name_source: FieldSource,
    /// How to extract the type annotation (if any)
    pub type_source: Option<FieldSource>,
}

/// Where to find a piece of data in a tree-sitter node.
#[derive(Clone, Copy)]
pub enum FieldSource {
    /// Use `child_by_field_name(name)`
    Field(&'static str),
    /// Use `child(index)` (positional)
    Child(usize),
    /// The node's own text
    NodeText,
}

/// Rules for determining whether a symbol is public/exported.
pub enum VisibilityRule {
    /// All top-level symbols are public (Python)
    AllPublic,
    /// Exported if name starts with uppercase (Go)
    UppercaseExported,
    /// Must have a specific child node kind (e.g., Rust's "visibility_modifier")
    HasChildNode(&'static str),
    /// Must NOT have a child node with specific text values (Java/Kotlin: skip "private")
    NoAccessModifier {
        node_kind: &'static str,
        blocked_values: &'static [&'static str],
    },
    /// Must be wrapped in an export_statement (TypeScript/JavaScript)
    ExportWrapped,
    /// Name must not start with a prefix (Perl: skip "_" prefix)
    NoPrefix(&'static str),
}

/// How to build the signature string for a function/method.
pub enum SignatureStyle {
    /// `keyword name(params) -> return_type` (Python: `def foo(x: int) -> str`)
    KeywordBased(&'static str),
    /// Extract from source: start of node to end of return type or params (Rust, Go)
    SourceSpan,
    /// `keyword name` for non-function types (e.g., `class Foo`, `interface Bar`)
    TypeKeyword(&'static str),
}

/// How methods are found inside class/container bodies.
pub struct MethodSpec {
    /// Field name to access the class body (e.g., "body", "class_body")
    pub body_field: &'static str,
    /// Node kinds inside the body that represent methods
    pub method_node_kinds: &'static [&'static str],
    /// Self/this parameter names to filter out
    pub self_param_names: &'static [&'static str],
    /// Self/this parameter node kinds to skip entirely (e.g., Rust's "self_parameter")
    pub self_param_kinds: &'static [&'static str],
    /// Visibility rule for methods (may differ from top-level)
    pub visibility: Option<MethodVisibility>,
}

/// Visibility filtering for methods within a class.
pub enum MethodVisibility {
    /// Skip methods whose name starts with a prefix (Python: "_")
    SkipPrefix(&'static str),
    /// Skip methods with a specific accessibility modifier value
    SkipAccessModifier {
        node_kind: &'static str,
        blocked_values: &'static [&'static str],
    },
    /// Skip methods whose name starts with a char (TS: "#" for private fields)
    SkipNamePrefix(char),
}

/// Complete specification for extracting symbols from a language.
pub struct LanguageSpec {
    /// File extensions this spec handles (e.g., &["py"], &["java", "kt"])
    pub extensions: &'static [&'static str],

    /// Returns the tree-sitter Language for parsing
    pub ts_language: fn() -> tree_sitter::Language,

    // --- Top-level node extraction ---
    /// Which top-level node types to extract, and what SymbolKind they map to
    pub definition_nodes: &'static [NodeMapping],

    // --- Name extraction ---
    /// Field name to get the symbol name (almost always "name")
    pub name_field: &'static str,

    // --- Parameters ---
    /// Field name for the parameters node (usually "parameters")
    pub parameters_field: &'static str,
    /// How to extract individual parameters
    pub param_specs: &'static [ParamSpec],

    // --- Return type ---
    /// Field name for return type (None if language doesn't have return types in AST)
    pub return_type_field: Option<&'static str>,
    /// Whether the return type node wraps the actual type (TS includes ":" in return_type)
    pub return_type_unwrap_colon: bool,

    // --- Visibility ---
    pub visibility: VisibilityRule,

    // --- Signatures ---
    pub fn_signature: SignatureStyle,

    // --- Class/container methods ---
    pub method_spec: Option<MethodSpec>,

    // --- Special init/constructor method name ---
    /// Constructor name that should always be extracted even if it matches skip prefix
    /// (e.g., "__init__" for Python)
    pub constructor_name: Option<&'static str>,
}
