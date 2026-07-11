;; v0 plugin proof fixture (hand-authored; compiled to binary wasm by the test
;; harness with the `wat` crate). It exercises the whole v0 calling convention:
;;   1. a read-only query   (reticle.query_shapes)  -> ReadDocument
;;   2. a staged edit funnel (reticle.stage_edit)    -> StageEdit
;;
;; It reads the shape count of cell "TOP", then stages an AddShape whose y1 corner
;; is that count, so the applied shape proves the query returned the real value and
;; that it flowed through the command/undo funnel.
(module
  (import "reticle" "query_shapes" (func $query_shapes (param i32 i32) (result i32)))
  (import "reticle" "stage_edit"   (func $stage_edit   (param i32 i32) (result i32)))
  (memory (export "memory") 1)

  ;; Cell name "TOP" at offset 0 (argument to query_shapes).
  (data (i32.const 0) "TOP")

  ;; A v0 AddShape record at offset 16:
  ;;   opcode 0x01 | name_len 3 | "TOP" | layer 1 | datatype 0 |
  ;;   x0 0 | y0 0 | x1 100 | y1 200   (y1 is overwritten with the query result)
  (data (i32.const 16)
    "\01\03\00TOP\01\00\00\00\00\00\00\00\00\00\00\00\64\00\00\00\c8\00\00\00")

  (func (export "run")
    (local $n i32)
    ;; read-only query: number of shapes in cell "TOP"
    (local.set $n (call $query_shapes (i32.const 0) (i32.const 3)))
    ;; write the queried count into the record's y1 field (absolute offset 38)
    (i32.store align=1 (i32.const 38) (local.get $n))
    ;; stage the AddShape edit (26-byte record at offset 16)
    (drop (call $stage_edit (i32.const 16) (i32.const 26)))
  )
)
