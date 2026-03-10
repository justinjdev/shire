; Scala symbol extraction queries

; Classes
(class_definition
  name: (identifier) @name) @definition.class

; Objects (singleton)
(object_definition
  name: (identifier) @name) @definition.class

; Traits
(trait_definition
  name: (identifier) @name) @definition.trait

; Enums (Scala 3)
(enum_definition
  name: (identifier) @name) @definition.enum

; Type aliases
(type_definition
  name: (type_identifier) @name) @definition.type

; Top-level function definitions
(function_definition
  name: (identifier) @name) @definition.function

; Top-level function declarations
(function_declaration
  name: (identifier) @name) @definition.function

; Methods inside class/object/trait/enum bodies
(template_body
  (function_definition
    name: (identifier) @name) @definition.method)

(template_body
  (function_declaration
    name: (identifier) @name) @definition.method)

; Methods inside enum bodies
(enum_body
  (function_definition
    name: (identifier) @name) @definition.method)
