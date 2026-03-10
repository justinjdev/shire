; Top-level functions (not inside impl blocks)
(source_file
  (function_item
    name: (identifier) @name) @definition.function)

; Structs
(struct_item
  name: (type_identifier) @name) @definition.struct

; Enums
(enum_item
  name: (type_identifier) @name) @definition.enum

; Traits
(trait_item
  name: (type_identifier) @name) @definition.trait

; Impl methods
(impl_item
  body: (declaration_list
    (function_item
      name: (identifier) @name) @definition.method))
