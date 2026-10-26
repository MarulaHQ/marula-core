;; Imports WASI. A WASI instance would hand a plugin file descriptors, paths,
;; sockets and the environment, which is the whole surface the v1 ABI denies; the
;; module never loads.
(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 0) (i32.const 0)))
    (i64.const 0)))
