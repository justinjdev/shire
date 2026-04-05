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

; Reference: calls
(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    property: (property_identifier) @name)) @reference.call

; Suppression-only: match the name nodes of non-export-wrapped type-like
; declarations so they seed def_name_ranges (suppressing a Type self-ref at
; the declaration site). The main export-wrapped patterns above already match
; exported variants; these duplicates are deduped by seen_def_ranges and
; non-exported variants are filtered to None in post_process. Without these,
; `interface Foo {}` (non-exported) leaks `Foo` as a Type ref on its own line
; because `(type_identifier) @reference.type` below matches the interface's
; name node.
(class_declaration
  name: (type_identifier) @name) @definition.class
(interface_declaration
  name: (type_identifier) @name) @definition.interface
(type_alias_declaration
  name: (type_identifier) @name) @definition.type

; Reference: type usages
(type_identifier) @name @reference.type

; Reference: imports (named imports) — `import { Foo } from "./m"`
(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @name @reference.import))))

; Reference: imports (default import) — `import Foo from "./m"`
(import_statement
  (import_clause
    (identifier) @name @reference.import))

; Reference: imports (namespace import) — `import * as Foo from "./m"`
(import_statement
  (import_clause
    (namespace_import
      (identifier) @name @reference.import)))

; Reference: superclass (extends on class)
(class_declaration
  (class_heritage
    (extends_clause
      value: (identifier) @name @reference.impl)))

; Reference: implemented interfaces
(class_declaration
  (class_heritage
    (implements_clause
      (type_identifier) @name @reference.impl)))

; Reference: interface extends (`interface Foo extends Bar, Baz {}`)
(interface_declaration
  (extends_type_clause
    (type_identifier) @name @reference.impl))
