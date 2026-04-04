; Gleam symbol extraction queries

; Functions: pub fn name(params) -> Type { ... }
; Also matches external functions (preceded by @external attribute but same node type)
(function
  name: (identifier) @name) @definition.function

; Type definition: pub type Name { Constructor1 Constructor2 }
; Includes custom types and opaque types
(type_definition
  (type_name
    name: (type_identifier) @name)) @definition.class

; Type alias: pub type Name = OtherType
(type_alias
  (type_name
    name: (type_identifier) @name)) @definition.type

; Constant: pub const name: Type = value
(constant
  name: (identifier) @name) @definition.constant
