;; One line over the 100-line invocation budget. The over-budget call has no
;; return value, so the guest never learns; the host drops the line and the
;; invocation still succeeds with 100 captured lines.
(module
  (import "marula" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 512) "line")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (local $i i32)
    (block $done
      (loop $lines
        (br_if $done (i32.ge_u (local.get $i) (i32.const 101)))
        (call $log (i32.const 3) (i32.const 512) (i32.const 4))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lines)))
    (i64.const 0)))
