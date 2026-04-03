; Function definitions via <- assignment
(binary_operator
  lhs: (identifier) @name
  operator: "<-"
  rhs: (function_definition)) @definition.function

; Function definitions via = assignment
(binary_operator
  lhs: (identifier) @name
  operator: "="
  rhs: (function_definition)) @definition.function

; S4/R5 class definitions: setClass("Name", ...) / setRefClass("Name", ...)
; post_process filters to setClass/setRefClass calls only
(call
  function: (identifier) @_fn
  arguments: (arguments
    .
    (argument
      value: (string
        (string_content) @name)))) @definition.class

; R6 class definitions: Name <- R6Class(...)
; post_process filters to R6Class calls only
(binary_operator
  lhs: (identifier) @name
  operator: "<-"
  rhs: (call)) @definition.class
