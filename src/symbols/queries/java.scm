; Java symbol extraction queries

; Classes
(class_declaration
  name: (identifier) @name) @definition.class

; Interfaces
(interface_declaration
  name: (identifier) @name) @definition.interface

; Enums
(enum_declaration
  name: (identifier) @name) @definition.enum

; Records (a record's body is itself a `class_body`, so its explicit methods
; already match the `class_body (method_declaration ...)` pattern below; this
; pattern captures the record type itself).
(record_declaration
  name: (identifier) @name) @definition.struct

; Methods inside class bodies
(class_body
  (method_declaration
    name: (identifier) @name) @definition.method)

; Fields inside class bodies (filtered to public static final constants in post_process hook)
(class_body
  (field_declaration
    declarator: (variable_declarator
      name: (identifier) @name)) @definition.constant)

; Methods inside interface bodies (implicitly public — see is_visible in java.rs)
(interface_body
  (method_declaration
    name: (identifier) @name) @definition.method)

; Constants inside interface bodies (implicitly public static final)
(interface_body
  (constant_declaration
    declarator: (variable_declarator
      name: (identifier) @name)) @definition.constant)

; Methods inside enum bodies (e.g. `enum Color { RED, GREEN; String label() {} }`)
(enum_body_declarations
  (method_declaration
    name: (identifier) @name) @definition.method)

; Enum constants (e.g. `RED` and `GREEN` in `enum Color { RED, GREEN; }`) —
; implicitly public static final, see effective_modifiers in java.rs.
(enum_body
  (enum_constant
    name: (identifier) @name) @definition.constant)

; Fields inside enum bodies (filtered to public static final constants in
; post_process, same as class_body fields above).
(enum_body_declarations
  (field_declaration
    declarator: (variable_declarator
      name: (identifier) @name)) @definition.constant)

; Reference: method calls
(method_invocation
  name: (identifier) @name) @reference.call

; Reference: type usages
(type_identifier) @name @reference.type

; Reference: imports
; Capture the terminal identifier of a scoped_identifier so
; `import java.util.List;` records as `List` (simple name), not
; `java.util.List`. symbol_refs lookup is exact-name, so the full
; qualified path would never match a user's search for `List`.
(import_declaration
  (scoped_identifier
    name: (identifier) @name)) @reference.import

; Bare identifier imports (rare) — record as-is.
(import_declaration
  (identifier) @name) @reference.import

; Reference: superclass (class extends)
(class_declaration
  (superclass
    (type_identifier) @name @reference.impl))

; Reference: implemented interfaces (class implements)
(class_declaration
  (super_interfaces
    (type_list
      (type_identifier) @name @reference.impl)))

; Reference: extended interfaces (interface extends)
(interface_declaration
  (extends_interfaces
    (type_list
      (type_identifier) @name @reference.impl)))
