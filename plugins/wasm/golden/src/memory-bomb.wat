;; Grows linear memory until the store's cap refuses the growth. With
;; trap_on_grow_failure set on the store, the failed grow is a trap and the
;; invocation ends as a guest trap rather than a guest that saw -1 and carried on.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (loop $grow
      (drop (memory.grow (i32.const 64)))
      (br $grow))
    (i64.const 0)))
