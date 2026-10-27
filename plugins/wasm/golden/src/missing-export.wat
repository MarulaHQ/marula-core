;; Missing `marula_free`. The ABI fixes three exports; a module that omits one is
;; not a guest this host can run, and the refusal happens at load.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (i64.const 0)))
