; Dart symbol extraction queries (tree-sitter-dart 0.1.0 grammar)

; Classes
(class_declaration
  name: (identifier) @name) @definition.class

; Mixins
(mixin_declaration
  name: (identifier) @name) @definition.class

; Extensions
(extension_declaration
  name: (identifier) @name) @definition.class

; Enums
(enum_declaration
  name: (identifier) @name) @definition.enum

; Top-level functions
(source_file
  (function_signature
    name: (identifier) @name) @definition.function)

; Top-level getters (direct children of source_file)
(source_file
  (getter_signature
    name: (identifier) @name) @definition.function)

; Top-level setters (direct children of source_file)
(source_file
  (setter_signature
    name: (identifier) @name) @definition.function)

; Methods inside class/mixin/extension bodies (with function_body sibling)
(method_signature
  (function_signature
    name: (identifier) @name)) @definition.method

; Abstract/external methods (inside declaration node, no function_body)
(class_body
  (class_member
    (declaration
      (function_signature
        name: (identifier) @name)) @definition.method))

; Abstract/external getters (inside declaration node)
(class_body
  (class_member
    (declaration
      (getter_signature
        name: (identifier) @name)) @definition.method))

; Abstract/external setters (inside declaration node)
(class_body
  (class_member
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

; Const constructors
(constant_constructor_signature
  name: (identifier) @name) @definition.method

; Factory constructors
(factory_constructor_signature
  name: (identifier) @name) @definition.method

; Redirecting factory constructors
(redirecting_factory_constructor_signature
  name: (identifier) @name) @definition.method

; Typedefs
(type_alias
  (type_identifier) @name) @definition.type
