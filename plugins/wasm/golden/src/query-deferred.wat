;; `read_model_query` id 1 with `tickets.read` granted. The gate passes and the
;; handler refuses `plugin_not_implemented` (13): the fetch needs core-db and the
;; caller's transaction, which the ABI declines to fake.
(module
  (import "marula" "read_model_query" (func $read (param i32 i32 i32) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 512) "{\"id\":\"01J\"}")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $read (i32.const 1) (i32.const 512) (i32.const 12))))
