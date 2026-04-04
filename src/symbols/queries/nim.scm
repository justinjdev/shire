; Nim symbol extraction queries

; Proc declarations (exported)
(proc_declaration
  name: (exported_symbol
    (identifier) @name)) @definition.function

; Proc declarations (private — filtered by is_visible hook)
(proc_declaration
  name: (identifier) @name) @definition.function

; Func declarations (exported)
(func_declaration
  name: (exported_symbol
    (identifier) @name)) @definition.function

; Func declarations (private)
(func_declaration
  name: (identifier) @name) @definition.function

; Method declarations (exported)
(method_declaration
  name: (exported_symbol
    (identifier) @name)) @definition.method

; Method declarations (private)
(method_declaration
  name: (identifier) @name) @definition.method

; Iterator declarations (exported)
(iterator_declaration
  name: (exported_symbol
    (identifier) @name)) @definition.function

; Iterator declarations (private)
(iterator_declaration
  name: (identifier) @name) @definition.function

; Template declarations (exported)
(template_declaration
  name: (exported_symbol
    (identifier) @name)) @definition.function

; Template declarations (private)
(template_declaration
  name: (identifier) @name) @definition.function

; Macro declarations (exported)
(macro_declaration
  name: (exported_symbol
    (identifier) @name)) @definition.function

; Macro declarations (private)
(macro_declaration
  name: (identifier) @name) @definition.function

; Converter declarations (exported)
(converter_declaration
  name: (exported_symbol
    (identifier) @name)) @definition.function

; Converter declarations (private)
(converter_declaration
  name: (identifier) @name) @definition.function

; Type declarations (type_symbol_declaration holds the name)
; Captured at type_declaration level so post_process can inspect sibling enum/object nodes.
(type_section
  (type_declaration
    (type_symbol_declaration
      name: (exported_symbol
        (identifier) @name)) @definition.type))

(type_section
  (type_declaration
    (type_symbol_declaration
      name: (identifier) @name) @definition.type))

; Variable declarations inside const_section
(const_section
  (variable_declaration
    (symbol_declaration_list
      (symbol_declaration
        name: (exported_symbol
          (identifier) @name)))) @definition.constant)

(const_section
  (variable_declaration
    (symbol_declaration_list
      (symbol_declaration
        name: (identifier) @name))) @definition.constant)
