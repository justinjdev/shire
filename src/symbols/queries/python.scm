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
