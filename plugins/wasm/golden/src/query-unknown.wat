;; `read_model_query` with an id outside the closed table. The id check runs
;; before any capability question, so a guest cannot map the grants by varying
;; the id: the answer is `plugin_query_unknown` (11) whatever it holds.
(module
  (import "marula" "read_model_query" (func $read (param i32 i32 i32) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 512) "{}")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $read (i32.const 9) (i32.const 512) (i32.const 2))))
