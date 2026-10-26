;; `read_model_query` id 1 (`ticket.by_id`) from an install holding no read
;; capability, with the query known. The gate answers
;; `plugin_capability_not_granted` (5).
(module
  (import "marula" "read_model_query" (func $read (param i32 i32 i32) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 512) "{\"id\":\"01J\"}")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $read (i32.const 1) (i32.const 512) (i32.const 12))))
