; Nix symbol extraction queries
;
; Nix is expression-oriented — definitions are bindings (name = value)
; inside attribute sets or let expressions. All bindings are captured
; as definition.constant; the post_process hook reclassifies based on
; the value expression type (function, attrset, etc.).

; Attribute bindings: name = value;
; Captures the first identifier of the attrpath as the symbol name.
(binding
  attrpath: (attrpath
    attr: (identifier) @name)
  expression: (_)) @definition.constant
