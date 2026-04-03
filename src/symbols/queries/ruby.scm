; Ruby symbol extraction queries

; Module definitions
(module
  name: (constant) @name) @definition.class

; Class definitions
(class
  name: (constant) @name) @definition.class

; Instance method definitions
(method
  name: (identifier) @name) @definition.function

; Class method definitions (def self.foo)
(singleton_method
  name: (identifier) @name) @definition.function
