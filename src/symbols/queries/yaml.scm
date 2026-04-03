; Top-level mapping keys — plain scalar (e.g. name: value)
(document
  (block_node
    (block_mapping
      (block_mapping_pair
        key: (flow_node
          (plain_scalar
            (string_scalar) @name))) @definition.constant)))

; Top-level mapping keys — double-quoted (e.g. "on": true)
(document
  (block_node
    (block_mapping
      (block_mapping_pair
        key: (flow_node
          (double_quote_scalar) @name)) @definition.constant)))

; Top-level mapping keys — single-quoted (e.g. 'key': value)
(document
  (block_node
    (block_mapping
      (block_mapping_pair
        key: (flow_node
          (single_quote_scalar) @name)) @definition.constant)))
