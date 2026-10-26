;; `kv_get` with a namespace label the host will not compose. The namespace is
;; the install's own slot; the label inside it is still `[a-z0-9_.-]{1,64}`, and a
;; path-shaped label is refused with `plugin_kv_namespace_denied`. The guest
;; returns the refusal frame verbatim, so the corpus asserts on the bytes a PDK
;; guest would decode.
(module
  (import "marula" "kv_get" (func $kv_get (param i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 512) "../etc")
  (data (i32.const 544) "k")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $kv_get (i32.const 512) (i32.const 6) (i32.const 544) (i32.const 1))))
