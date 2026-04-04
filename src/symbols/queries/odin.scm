; Odin symbol extraction queries
; Uses `.` anchor to match only the first (name) identifier in each declaration.

; Struct declarations: Vector2 :: struct { ... }
(struct_declaration . (identifier) @name) @definition.class

; Enum declarations: Direction :: enum { ... }
(enum_declaration . (identifier) @name) @definition.enum

; Union declarations: Result :: union { ... }
(union_declaration . (identifier) @name) @definition.class

; Procedure declarations: add :: proc(a, b: int) -> int { ... }
(procedure_declaration . (identifier) @name) @definition.function

; Constant declarations: MAX_SIZE :: 1024
(const_declaration . (identifier) @name) @definition.constant
