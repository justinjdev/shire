; Clojure symbol extraction queries
; All top-level definitions are list_lit nodes; hooks filter by def keyword.

; Top-level list forms: first sym is the def keyword, second sym is the name.
(list_lit
  value: (sym_lit)
  value: (sym_lit) @name) @definition.function
