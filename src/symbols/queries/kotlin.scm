; Kotlin symbol extraction queries

; Classes (also matches interfaces and enum classes — post_process determines actual kind)
(class_declaration
  name: (identifier) @name) @definition.class

; Object declarations (singleton objects)
(object_declaration
  name: (identifier) @name) @definition.class

; Top-level functions
(source_file
  (function_declaration
    name: (identifier) @name) @definition.function)

; Methods inside class bodies
(class_body
  (function_declaration
    name: (identifier) @name) @definition.method)

; Methods inside enum class bodies
(enum_class_body
  (function_declaration
    name: (identifier) @name) @definition.method)
