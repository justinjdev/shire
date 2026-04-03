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

; Methods inside class/mixin/extension bodies (with function_body sibling)
(method_signature
  (function_signature
    name: (identifier) @name)) @definition.method

; Abstract/external methods (inside declaration node, no function_body)
(class_body
  (class_member_definition
    (declaration
      (function_signature
        name: (identifier) @name)) @definition.method))

; Abstract/external getters (inside declaration node)
(class_body
  (class_member_definition
    (declaration
      (getter_signature
        name: (identifier) @name)) @definition.method))

; Abstract/external setters (inside declaration node)
(class_body
  (class_member_definition
    (declaration
      (setter_signature
        name: (identifier) @name)) @definition.method))

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

; Const constructors (name is inside qualified node)
(constant_constructor_signature
  (qualified
    (identifier) @name)) @definition.method

; Factory constructors
(factory_constructor_signature
  (identifier) @name) @definition.method

; Redirecting factory constructors
(redirecting_factory_constructor_signature
  (identifier) @name) @definition.method

; Typedefs
(type_alias
  (type_identifier) @name) @definition.type
