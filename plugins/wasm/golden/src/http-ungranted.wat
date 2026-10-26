;; `http_request` from an install with no network capability at all. The gate
;; answers `plugin_capability_not_granted` before the profile is even looked at,
;; so a guest cannot learn which profiles an org consented to.
(module
  (import "marula" "http_request" (func $http (param i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 512) "vendor")
  (data (i32.const 544) "{\"method\":\"GET\",\"url\":\"https://api.example.com/v1\"}")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $http (i32.const 512) (i32.const 6) (i32.const 544) (i32.const 51))))
