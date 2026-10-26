;; A guest that never yields. Fuel is the cap that stops a tight loop: 200,000,000
;; fuel is about 200 ms of compiled code, inside the 250 ms epoch.
(module
  (memory (export "memory") 1)
  (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "marula_free") (param i32) (param i32))
  (func (export "marula_entry") (param i32) (param i32) (result i64)
    (loop $spin (br $spin))
    (i64.const 0)))
