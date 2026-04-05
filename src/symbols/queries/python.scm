; Top-level functions
(module
  (function_definition
    name: (identifier) @name) @definition.function)

; Classes
(class_definition
  name: (identifier) @name) @definition.class

; Methods (functions inside class body)
(class_definition
  body: (block
    (function_definition
      name: (identifier) @name) @definition.method))

; Reference: function/method calls
(call
  function: (identifier) @name) @reference.call

(call
  function: (attribute
    attribute: (identifier) @name)) @reference.call

; Reference: type annotations
(type
  (identifier) @name) @reference.type

; Reference: imports
(import_statement
  name: (dotted_name (identifier) @name)) @reference.import

(import_from_statement
  name: (dotted_name (identifier) @name)) @reference.import

; Reference: superclasses (impl)
(class_definition
  superclasses: (argument_list
    (identifier) @name @reference.impl))
