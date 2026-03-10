; Classes, structs, enums, actors (all use class_declaration)
; The hooks post_process will reclassify based on keyword children
(class_declaration
  name: (type_identifier) @name) @definition.class

; Protocols
(protocol_declaration
  name: (type_identifier) @name) @definition.interface

; Top-level functions
(source_file
  (function_declaration
    name: (simple_identifier) @name) @definition.function)

; Methods inside class/struct/enum/actor bodies
(class_body
  (function_declaration
    name: (simple_identifier) @name) @definition.method)

; Protocol method declarations
(protocol_body
  (protocol_function_declaration
    name: (simple_identifier) @name) @definition.method)

; Type aliases
(typealias_declaration
  name: (type_identifier) @name) @definition.type
