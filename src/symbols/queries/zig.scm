; Zig symbol extraction queries

; Function declarations
(function_declaration
  name: (identifier) @name) @definition.function

; Variable/const declarations (structs, enums, unions defined via const assignment)
; The hooks post_process reclassifies to Struct/Enum based on the expression child.
; Note: variable_declaration has no `name` field — the identifier is an unnamed child.
(variable_declaration
  (identifier) @name) @definition.constant

; Test declarations
(test_declaration) @definition.function
