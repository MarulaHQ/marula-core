;; Two clock reads in one invocation. The guest has no clock and no WASI, so
;; this is the only time source it has; it returns "now" when the second read
;; is not before the first.
(module
  (import "marula" "now" (func $now (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 512) "now")
  (data (i32.const 520) "back")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (local $first i64)
    (local.set $first (call $now))
    (if (result i64)
      (i64.lt_s (call $now) (local.get $first))
      (then (i64.const 2233382993923))   ;; ptr 520, len 3: "back"
      (else (i64.const 2199023255555))))) ;; ptr 512, len 3: "now"
