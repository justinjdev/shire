; PHP symbol extraction queries

; Top-level functions
(function_definition
  name: (name) @name) @definition.function

; Classes
(class_declaration
  name: (name) @name) @definition.class

; Interfaces
(interface_declaration
  name: (name) @name) @definition.interface

; Traits (treat as trait kind)
(trait_declaration
  name: (name) @name) @definition.trait

; Enums
(enum_declaration
  name: (name) @name) @definition.enum

; Methods inside class/interface/trait/enum bodies
(declaration_list
  (method_declaration
    name: (name) @name) @definition.method)

; Constants inside class bodies
(declaration_list
  (const_declaration
    (const_element
      (name) @name)) @definition.constant)
