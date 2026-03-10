; C symbol extraction queries

; Function definitions — name is inside declarator chain
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @definition.function

; Struct definitions
(struct_specifier
  name: (type_identifier) @name) @definition.struct

; Enum definitions
(enum_specifier
  name: (type_identifier) @name) @definition.enum

; Typedef type aliases — declarator may be type_identifier or primitive_type
(type_definition
  declarator: (type_identifier) @name) @definition.type

(type_definition
  declarator: (primitive_type) @name) @definition.type
