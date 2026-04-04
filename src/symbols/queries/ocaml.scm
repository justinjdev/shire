; OCaml symbol extraction queries
; Works with both LANGUAGE_OCAML (.ml) and LANGUAGE_OCAML_INTERFACE (.mli)

; Let bindings (functions and values — post_process reclassifies)
(value_definition
  (let_binding
    pattern: (value_name) @name) @definition.function)

; Type definitions
(type_definition
  (type_binding
    name: (type_constructor) @name) @definition.type)

; Module definitions
(module_definition
  (module_binding
    (module_name) @name) @definition.module)

; Module type definitions (signatures)
(module_type_definition
  (module_type_name) @name) @definition.interface

; Class definitions
(class_definition
  (class_binding
    (class_name) @name) @definition.class)

; Method definitions (.ml)
(method_definition
  (method_name) @name) @definition.method

; Exception definitions
(exception_definition
  (constructor_declaration
    (constructor_name) @name) @definition.type)

; External declarations
(external
  (value_name) @name) @definition.function

; Value specifications (.mli val declarations)
(value_specification
  (value_name) @name) @definition.function

; Method specifications (.mli)
(method_specification
  (method_name) @name) @definition.method
