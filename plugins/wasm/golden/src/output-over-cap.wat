;; One byte over the output cap: a plausible pointer and length, refused for the
;; length alone. This is the boundary the cap is drawn at.
(module
  (memory (export "memory") 17)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    ;; ptr 1024, len 1048577
    (i64.const 4398047559681)))
