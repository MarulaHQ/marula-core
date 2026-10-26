;; `marula_entry` with the wrong result type. The packing convention is an i64;
;; an i32 return would make the pointer and length ambiguous, so the module is
;; refused at load with the expected signature named.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i32)
    (i32.const 0)))
