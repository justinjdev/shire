; Elixir symbol extraction queries
; All definitions use generic `call` nodes; hooks filter by keyword.

; Module/protocol definitions: defmodule Foo.Bar do...end
(call
  target: (identifier)
  (arguments
    (alias) @name)) @definition.class

; Function/macro with parameters: def foo(a, b) do...end
(call
  target: (identifier)
  (arguments
    (call
      target: (identifier) @name))) @definition.function

; Function/macro with parameters and guard: def foo(a) when ... do...end
(call
  target: (identifier)
  (arguments
    (binary_operator
      left: (call
        target: (identifier) @name)))) @definition.function

; Function/macro without parameters (do-block): def foo do...end
(call
  target: (identifier)
  (arguments
    . (identifier) @name .)
  (do_block)) @definition.function

; Function/macro without parameters (one-liner): def foo, do: :ok
(call
  target: (identifier)
  (arguments
    (identifier) @name
    (keywords))) @definition.function

; @type simple: @type name :: type
(unary_operator
  operand: (call
    target: (identifier)
    (arguments
      (binary_operator
        left: (identifier) @name)))) @definition.type

; @type parameterized / @callback: @type name(a) :: type, @callback name(a) :: type
(unary_operator
  operand: (call
    target: (identifier)
    (arguments
      (binary_operator
        left: (call
          target: (identifier) @name))))) @definition.type
