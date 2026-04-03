; Table definitions
(create_table
  (object_reference
    name: (identifier) @name)) @definition.class

; View definitions
(create_view
  (object_reference
    name: (identifier) @name)) @definition.interface

; Materialized view definitions
(create_materialized_view
  (object_reference
    name: (identifier) @name)) @definition.interface

; Function definitions
(create_function
  (object_reference
    name: (identifier) @name)) @definition.function

; Index definitions
; Note: the grammar uses `column` as the field name for the index name (a naming quirk)
(create_index
  column: (identifier) @name) @definition.constant

; Trigger definitions (anchored to first object_reference after TRIGGER keyword)
(create_trigger
  (keyword_trigger)
  .
  (object_reference
    name: (identifier) @name)) @definition.function

; Type definitions
(create_type
  (object_reference
    name: (identifier) @name)) @definition.type
