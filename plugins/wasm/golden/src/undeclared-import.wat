;; Imports an entry the ABI does not declare. `secret_get` is deferred by
;; ruling (ADR 0023 Decision 3), so it is absent from the registry: the module is
;; refused at load, before it can run, rather than refused per call.
(module
  (import "marula" "secret_get" (func $secret (param i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $secret (i32.const 0) (i32.const 1) (i32.const 2) (i32.const 3))))
