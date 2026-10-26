;; Claims the largest length a 32-bit field can hold. The host checks the
;; claimed length against the output cap BEFORE it copies a byte, so this is
;; refused without the host allocating anything.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (i64.const 4294967295)))
