; Protobuf symbol extraction queries

; Top-level message (restricted to source_file to avoid matching nested messages)
(source_file
  (message
    (message_name) @name) @definition.struct)

; Top-level service
(service
  (service_name) @name) @definition.interface

; RPC inside service
(rpc
  (rpc_name) @name) @definition.method

; Top-level enum (restricted to source_file to avoid matching nested enums)
(source_file
  (enum
    (enum_name) @name) @definition.enum)

; Nested message inside message_body
(message_body
  (message
    (message_name) @name) @definition.struct)

; Nested enum inside message_body
(message_body
  (enum
    (enum_name) @name) @definition.enum)

; Oneof inside message_body
(message_body
  (oneof
    (identifier) @name) @definition.type)
