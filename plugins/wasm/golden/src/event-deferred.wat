;; `emit_event` with the capability granted and the type declared. The gate
;; passes in full and the handler refuses with `plugin_not_implemented` (13): the
;; publish needs the event registry and the core-events producer, and the ABI
;; says so instead of pretending a row was written.
(module
  (import "marula" "emit_event" (func $emit (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 512) "ticket.updated")
  (data (i32.const 560) "{}")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (local $code i32)
    (local.set $code
      (call $emit (i32.const 512) (i32.const 14) (i32.const 560) (i32.const 2)))
    (i32.store8 (i32.const 900) (local.get $code))
    (i64.const 3865470566401)))
