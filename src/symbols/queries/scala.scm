; Scala symbol extraction queries

; Classes
(class_definition
  name: (identifier) @name) @definition.class

; Objects (singleton)
(object_definition
  name: (identifier) @name) @definition.class

; Traits
(trait_definition
  name: (identifier) @name) @definition.trait

; Enums (Scala 3)
(enum_definition
  name: (identifier) @name) @definition.enum

; Type aliases
(type_definition
  name: (type_identifier) @name) @definition.type

; Top-level function definitions
(function_definition
  name: (identifier) @name) @definition.function

; Top-level function declarations
(function_declaration
  name: (identifier) @name) @definition.function

; Methods inside class/object/trait/enum bodies
(template_body
  (function_definition
    name: (identifier) @name) @definition.method)

(template_body
  (function_declaration
    name: (identifier) @name) @definition.method)

; Methods inside enum bodies
(enum_body
  (function_definition
    name: (identifier) @name) @definition.method)

; Reference: calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (field_expression
    field: (identifier) @name)) @reference.call

; Reference: type usages
(type_identifier) @name @reference.type

; Reference: imports (path: field is a sequence of identifier nodes)
(import_declaration
  path: (identifier) @name @reference.import)

; Reference: extends clause (superclass + traits via "with")
; tree-sitter only matches the first `type:` occurrence with a named field,
; so use positional matching to capture all type_identifiers in the clause.
(extends_clause
  (type_identifier) @name @reference.impl)

(extends_clause
  (generic_type
    (type_identifier) @name @reference.impl))
