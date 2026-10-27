;; The smallest conformant guest: three exports, one memory, no host calls.
;; Its entry returns an empty payload, which the host reads as "no output".
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (i64.const 0)))
