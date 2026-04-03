; Tables: [name] and [dotted.name]
(table (bare_key) @name) @definition.module
(table (dotted_key) @name) @definition.module

; Array of tables: [[name]] and [[dotted.name]]
(table_array_element (bare_key) @name) @definition.module
(table_array_element (dotted_key) @name) @definition.module

; Top-level key-value pairs (bare, dotted, and quoted keys)
(document (pair (bare_key) @name) @definition.constant)
(document (pair (dotted_key) @name) @definition.constant)
(document (pair (quoted_key) @name) @definition.constant)
