;; An allocator that refuses: a negative pointer is not addressable through the
;; ABI's 32-bit unsigned convention, so the host refuses the invocation instead
;; of writing at a wrapped address.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const -1))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (i64.const 0)))
