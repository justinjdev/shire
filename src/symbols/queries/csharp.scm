; C# symbol extraction queries

; Classes
(class_declaration
  name: (identifier) @name) @definition.class

; Structs
(struct_declaration
  name: (identifier) @name) @definition.struct

; Interfaces
(interface_declaration
  name: (identifier) @name) @definition.interface

; Enums
(enum_declaration
  name: (identifier) @name) @definition.enum

; Records (treat as class)
(record_declaration
  name: (identifier) @name) @definition.class

; Methods inside class/struct/record bodies
(declaration_list
  (method_declaration
    name: (identifier) @name) @definition.method)

; Interface methods
(declaration_list
  (method_declaration
    name: (identifier) @name) @definition.method)

; Constants (field declarations that are const or static readonly)
(declaration_list
  (field_declaration
    (variable_declaration
      (variable_declarator
        (identifier) @name))) @definition.constant)
