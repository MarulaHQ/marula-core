;; One log line of 4097 bytes: a byte over the 4 KiB per-line cap. The line is
;; dropped and the invocation succeeds, which is the documented answer for a
;; host function with no return value.
(module
  (import "marula" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 2)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 8192))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $log (i32.const 3) (i32.const 0) (i32.const 4097))
    (i64.const 0)))
