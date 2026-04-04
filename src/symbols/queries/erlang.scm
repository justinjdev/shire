; Erlang symbol extraction queries

; Module declaration: -module(name).
(module_attribute
  name: (atom) @name) @definition.module

; Function declarations: name(args) -> body.
; Matches each function_clause inside a fun_decl.
; Multiple clauses are deduplicated in post_process.
(fun_decl
  clause: (function_clause
    name: (atom) @name)) @definition.function

; Type definitions: -type name() :: type.
(type_alias
  name: (type_name
    name: (atom) @name)) @definition.type

; Record declarations: -record(name, {fields}).
(record_decl
  name: (atom) @name) @definition.class

; Callback declarations: -callback name(types) -> type.
(callback
  fun: (atom) @name) @definition.method

; Macro definitions: -define(NAME, value).
(pp_define
  lhs: (macro_lhs
    name: (var) @name)) @definition.constant
