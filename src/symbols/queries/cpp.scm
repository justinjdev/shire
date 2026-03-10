; C++ symbol extraction queries

; Top-level function definitions
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition.function

; Also match qualified identifiers for methods defined outside class
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier) @name)) @definition.function

; Classes
(class_specifier
  name: (type_identifier) @name) @definition.class

; Structs
(struct_specifier
  name: (type_identifier) @name) @definition.struct

; Enums
(enum_specifier
  name: (type_identifier) @name) @definition.enum

; Namespaces (as modules)
(namespace_definition
  name: (namespace_identifier) @name) @definition.module

; Type aliases (using =)
(alias_declaration
  name: (type_identifier) @name) @definition.type

; Methods inside class/struct bodies
(field_declaration_list
  (function_definition
    declarator: (function_declarator
      declarator: (field_identifier) @name)) @definition.method)
