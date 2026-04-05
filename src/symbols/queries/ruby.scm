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

; Reference: method calls with identifier (regular methods)
(call
  method: (identifier) @name) @reference.call

; Reference: method calls with constant (e.g. ClassName.method or MODULE_CONST())
(call
  method: (constant) @name) @reference.call

; Reference: constant references (type-like usage)
(constant) @name @reference.type

; Reference: require / require_relative / load (import references)
(call
  method: (identifier) @method_name
  arguments: (argument_list
    (string (string_content) @name))
  (#any-of? @method_name "require" "require_relative" "load")) @reference.import

; Reference: include / prepend / extend mixins (impl references)
(call
  method: (identifier) @method_name
  arguments: (argument_list (constant) @name @reference.impl)
  (#any-of? @method_name "include" "prepend" "extend"))

; Reference: superclass in class definition
(class
  superclass: (superclass
    (constant) @name @reference.impl))
