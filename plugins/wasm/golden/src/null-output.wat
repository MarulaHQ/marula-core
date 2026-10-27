;; The zero word. It is not a pointer to anything, so the host has to decide
;; what "no output" means; the corpus pins that answer rather than leaving it to
;; a reader of the reply struct.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (i64.const 0)))
