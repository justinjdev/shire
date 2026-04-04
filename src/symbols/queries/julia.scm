; Julia symbol extraction queries

; Function definitions: function name(args) ... end
; Without return type annotation: signature > call_expression > identifier
(function_definition
  (signature
    (call_expression
      (identifier) @name))) @definition.function

; Function definitions with return type: function name(args)::Type ... end
; With return type annotation: signature > typed_expression > call_expression > identifier
(function_definition
  (signature
    (typed_expression
      (call_expression
        (identifier) @name)))) @definition.function

; Short function form: f(x) = expr (these are assignment nodes with call on left)
(assignment
  (call_expression
    (identifier) @name)) @definition.function

; Short function form with return type: f(x)::T = expr
(assignment
  (typed_expression
    (call_expression
      (identifier) @name))) @definition.function

; Struct definitions: struct Name ... end / mutable struct Name ... end
(struct_definition
  (type_head
    (identifier) @name)) @definition.class

; Abstract type: abstract type Name end
(abstract_definition
  (type_head
    (identifier) @name)) @definition.interface

; Module: module Name ... end
(module_definition
  name: (identifier) @name) @definition.module

; Macro: macro name(args) ... end
(macro_definition
  (signature
    (call_expression
      (identifier) @name))) @definition.function

; Const: const NAME = value
(const_statement
  (assignment
    (identifier) @name)) @definition.constant
