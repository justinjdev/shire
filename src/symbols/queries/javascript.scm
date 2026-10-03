; JavaScript symbol extraction queries
; Exported and module-level declarations; the visibility hook tags the
; unexported ones private.

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

; Module-level declarations that are not exported (private to the module;
; the visibility hook tags them, and recognises names exported later through
; an `export { ... }` clause). Each node also matches nothing above, or the
; same range as a pattern above (deduped by seen_def_ranges).
(program
  (function_declaration
    name: (identifier) @name) @definition.function)

(program
  (class_declaration
    name: (identifier) @name) @definition.class)

(program
  (class_declaration
    body: (class_body
      (method_definition
        name: (property_identifier) @name) @definition.method)))

(program
  (lexical_declaration
    (variable_declarator
      name: (identifier) @name) @definition.constant))

; `#private` methods, in an exported or a module-level class
(export_statement
  (class_declaration
    body: (class_body
      (method_definition
        name: (private_property_identifier) @name) @definition.method)))

(program
  (class_declaration
    body: (class_body
      (method_definition
        name: (private_property_identifier) @name) @definition.method)))

; Reference: calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    property: (property_identifier) @name)) @reference.call

; Reference: imports (named imports)
(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @name @reference.import))))

; Reference: superclass
(class_declaration
  (class_heritage
    (identifier) @name @reference.impl))
