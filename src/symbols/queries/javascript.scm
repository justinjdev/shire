; JavaScript symbol extraction queries
; All patterns require export_statement wrapper to only capture exported symbols.

; Exported functions
(export_statement
  (function_declaration
    name: (identifier) @name) @definition.function)

; Exported classes
(export_statement
  (class_declaration
    name: (identifier) @name) @definition.class)

; Methods inside exported classes
(export_statement
  (class_declaration
    body: (class_body
      (method_definition
        name: (property_identifier) @name) @definition.method)))

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
    name: (identifier) @name) @definition.class)
