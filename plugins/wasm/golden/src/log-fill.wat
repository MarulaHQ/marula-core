;; Fills the log budget exactly: 100 lines of "line". One more would be refused
;; with no in-band channel, so 100 is the last line that is captured.
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
        (br_if $done (i32.ge_u (local.get $i) (i32.const 100)))
        (call $log (i32.const 3) (i32.const 512) (i32.const 4))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lines)))
    (i64.const 0)))
