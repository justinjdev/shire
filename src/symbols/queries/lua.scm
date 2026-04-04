; Lua symbol extraction queries

; Global and local function declarations
(function_declaration
  name: (identifier) @name) @definition.function

; Module function declarations (function M.foo() end)
(function_declaration
  name: (dot_index_expression
    field: (identifier) @name)) @definition.function

; Method declarations (function M:foo() end)
(function_declaration
  name: (method_index_expression
    method: (identifier) @name)) @definition.method

; Assignment-style function definitions (foo = function() end, M.foo = function() end)
(assignment_statement
  (variable_list
    .
    name: [
      (identifier) @name
      (dot_index_expression
        field: (identifier) @name)
    ])
  (expression_list
    .
    value: (function_definition))) @definition.function
