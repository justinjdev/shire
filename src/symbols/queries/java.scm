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

; Methods inside class bodies
(class_body
  (method_declaration
    name: (identifier) @name) @definition.method)

; Fields inside class bodies (filtered to public static final constants in post_process hook)
(class_body
  (field_declaration
    declarator: (variable_declarator
      name: (identifier) @name)) @definition.constant)
