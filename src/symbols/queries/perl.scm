; Perl symbol extraction queries

; Package declarations
(package_statement
  name: (package) @name) @definition.class

; Subroutine definitions
(subroutine_declaration_statement
  name: (bareword) @name) @definition.function

; Reference: subroutine/function calls
(function_call_expression
  function: (function) @name) @reference.call

; Reference: use/require module imports
(use_statement
  module: (package) @name) @reference.import
