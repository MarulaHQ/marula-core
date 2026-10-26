;; Unbounded recursion. The engine's stack guard turns it into a trap; the
;; worker survives and the next invocation is unaffected.
(module
  (memory (export "memory") 1)
  (func $recurse (call $recurse))
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $recurse)
    (i64.const 0)))
