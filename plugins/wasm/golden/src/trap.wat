;; The smallest hostile guest: an explicit trap. No limit is reached, so the
;; refusal is a guest trap and not a budget.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (unreachable)
    (i64.const 0)))
