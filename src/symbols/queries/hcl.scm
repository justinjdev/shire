; HCL/Terraform block definitions (resource, variable, output, data, module, provider)
; All HCL blocks share the same grammar node type — the first identifier child
; determines the block kind. Filtering is handled by the post_process hook.
(block
  (identifier) @name) @definition.class
