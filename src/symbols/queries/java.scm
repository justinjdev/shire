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

; Reference: method calls
(method_invocation
  name: (identifier) @name) @reference.call

; Reference: type usages
(type_identifier) @name @reference.type

; Reference: imports
(import_declaration
  (scoped_identifier) @name) @reference.import

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
