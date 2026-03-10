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
