;; Returns the input bytes unchanged. Proves the payload crosses the boundary
;; both ways at the size the corpus asks for, through a bump allocator that
;; grows the guest's memory as the input arrives.
(module
  (memory (export "memory") 4)
  (global $bump (mut i32) (i32.const 1024))

  (func (export "marula_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $bump))
    (global.set $bump
      (i32.and
        (i32.add (i32.add (global.get $bump) (local.get $len)) (i32.const 7))
        (i32.const -8)))
    ;; Grow until the allocation can fit. A refusal to grow traps, which is the
    ;; module's own fault and the host reports it as a guest trap.
    (block $fits
      (br_if $fits
        (i32.le_u
          (i32.add (local.get $ptr) (local.get $len))
          (i32.shl (memory.size) (i32.const 16))))
      (loop $grow
        (drop (memory.grow (i32.const 1)))
        (br_if $grow
          (i32.gt_u
            (i32.add (local.get $ptr) (local.get $len))
            (i32.shl (memory.size) (i32.const 16))))))
    (local.get $ptr))

  (func (export "marula_free") (param i32) (param i32))

  (func (export "marula_entry") (param $ptr i32) (param $len i32) (result i64)
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
      (i64.extend_i32_u (local.get $len)))))
