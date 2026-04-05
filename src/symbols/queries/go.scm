; Go symbol extraction queries

; Functions
(function_declaration
  name: (identifier) @name) @definition.function

; Methods
(method_declaration
  name: (field_identifier) @name) @definition.method

; Type declarations (struct, interface, and other types)
; The hooks post_process reclassifies to Struct/Interface based on the type child.
(type_declaration
  (type_spec
    name: (type_identifier) @name) @definition.type)

; Reference: function/method calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (selector_expression
    field: (field_identifier) @name)) @reference.call

; Reference: type usage (parameters, return types, struct fields)
(type_identifier) @name @reference.type

; Reference: imports (the import path string)
(import_spec
  path: (interpreted_string_literal) @name) @reference.import
