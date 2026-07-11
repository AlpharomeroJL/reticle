;; Memory-limit proof fixture (hand-authored; compiled by the test harness). It
;; imports nothing, so it needs no permissions. Its entry tries to grow the linear
;; memory well past the host's configured cap; when the host limiter denies the
;; growth, memory.grow returns -1 and the plugin traps, proving the cap is
;; enforced at run time.
(module
  (memory (export "memory") 1)
  (func (export "run")
    (if (i32.eq (memory.grow (i32.const 100)) (i32.const -1))
      (then unreachable))
  )
)
