; Haskell symbol extraction queries

; Top-level function definitions
(function
  name: (variable) @name) @definition.function

; Data type declarations
(data_type
  name: (name) @name) @definition.type

; Newtype declarations
(newtype
  name: (name) @name) @definition.type

; Type alias declarations (grammar spells it "synomym" — known typo)
(type_synomym
  name: (name) @name) @definition.type

; Type class declarations
(class
  name: (name) @name) @definition.trait

; Type class method signatures (signatures are direct children of class_declarations)
(class_declarations
  (signature
    (variable) @name) @definition.method)
