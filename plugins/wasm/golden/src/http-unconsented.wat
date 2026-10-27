;; `http_request` from an install that holds the read capability but consented to
;; no egress profile. The capability passes; the profile does not, and the answer
;; is `plugin_egress_profile_missing` rather than a socket.
(module
  (import "marula" "http_request" (func $http (param i32 i32 i32 i32) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 512) "vendor")
  (data (i32.const 544) "{\"method\":\"GET\",\"url\":\"https://api.example.com/v1\"}")
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (call $http (i32.const 512) (i32.const 6) (i32.const 544) (i32.const 51))))
