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

; Fields inside class bodies (constants: public static final)
(class_body
  (field_declaration
    declarator: (variable_declarator
      name: (identifier) @name)) @definition.constant)
