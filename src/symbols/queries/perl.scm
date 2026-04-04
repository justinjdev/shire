; Perl symbol extraction queries

; Package declarations
(package_statement
  name: (package) @name) @definition.class

; Subroutine definitions
(subroutine_declaration_statement
  name: (bareword) @name) @definition.function
