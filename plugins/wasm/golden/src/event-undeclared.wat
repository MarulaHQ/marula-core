;; `emit_event` with the capability granted but the event type absent from the
;; manifest's declaration. The declaration gate runs before the publish, so the
;; answer is `plugin_event_type_not_declared` (10) and not the stub's answer.
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
    (i64.const 3865470566401))) ;; ptr 900, len 1
