; Dart symbol extraction queries (tree-sitter-dart 0.0.4 grammar)

; Classes
(class_definition
  name: (identifier) @name) @definition.class

; Mixins (post_process sets signature to "mixin Name")
(mixin_declaration
  (identifier) @name) @definition.class

; Extensions
(extension_declaration
  name: (identifier) @name) @definition.class

; Enums
(enum_declaration
  name: (identifier) @name) @definition.enum

; Top-level functions (wrapped in lambda_expression in this grammar)
(program
  (lambda_expression
    parameters: (function_signature
      name: (identifier) @name) @definition.function))

; Top-level getters (direct children of program)
(program
  (getter_signature
    name: (identifier) @name) @definition.function)

; Top-level setters (direct children of program)
(program
  (setter_signature
    name: (identifier) @name) @definition.function)

; Methods inside class/mixin/extension bodies
(method_signature
  (function_signature
    name: (identifier) @name)) @definition.method

; Getter methods
(method_signature
  (getter_signature
    name: (identifier) @name)) @definition.method

; Setter methods
(method_signature
  (setter_signature
    name: (identifier) @name)) @definition.method

; Constructors
(constructor_signature
  name: (identifier) @name) @definition.method

; Factory constructors
(factory_constructor_signature
  (identifier) @name) @definition.method

; Redirecting factory constructors
(redirecting_factory_constructor_signature
  (identifier) @name) @definition.method

; Typedefs
(type_alias
  (type_identifier) @name) @definition.type
