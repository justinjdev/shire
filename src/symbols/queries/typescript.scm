; TypeScript symbol extraction queries
; All patterns require export_statement wrapper to only capture exported symbols.

; Exported functions
(export_statement
  (function_declaration
    name: (identifier) @name) @definition.function)

; Exported function signatures (ambient declarations)
(export_statement
  (function_signature
    name: (identifier) @name) @definition.function)

; Exported classes
(export_statement
  (class_declaration
    name: (type_identifier) @name) @definition.class)

; Methods inside exported classes
(export_statement
  (class_declaration
    body: (class_body
      (method_definition
        name: (property_identifier) @name) @definition.method)))

; Exported interfaces
(export_statement
  (interface_declaration
    name: (type_identifier) @name) @definition.interface)

; Exported type aliases
(export_statement
  (type_alias_declaration
    name: (type_identifier) @name) @definition.type)

; Exported enums
(export_statement
  (enum_declaration
    name: (identifier) @name) @definition.enum)

; Exported constants (lexical_declaration with variable_declarator)
(export_statement
  (lexical_declaration
    (variable_declarator
      name: (identifier) @name) @definition.constant))

; Default export function
(export_statement
  "default"
  (function_declaration
    name: (identifier) @name) @definition.function)

; Default export class
(export_statement
  "default"
  (class_declaration
    name: (type_identifier) @name) @definition.class)
