;; v0 real-query proof fixture (hand-authored; compiled to binary wasm by the test
;; harness with the `wat` crate). It exercises the whole read-only query surface:
;;   reticle.query_shapes      -> ReadDocument
;;   reticle.query_selection   -> ReadSelection
;;   reticle.query_technology  -> ReadTechnology
;; then stages one AddShape per query, on distinct layers 1/2/3, whose y1 corner is
;; that query's result. The applied shapes prove each query returned the REAL value
;; from the pre-run snapshot (document shape count, resolved selection count, and the
;; active technology's dbu_per_micron) and that it flowed through the edit funnel.
(module
  (import "reticle" "query_shapes"     (func $query_shapes     (param i32 i32) (result i32)))
  (import "reticle" "query_selection"  (func $query_selection  (result i32)))
  (import "reticle" "query_technology" (func $query_technology (result i32)))
  (import "reticle" "stage_edit"       (func $stage_edit       (param i32 i32) (result i32)))
  (memory (export "memory") 1)

  ;; Cell name "TOP" at offset 0 (argument to query_shapes).
  (data (i32.const 0) "TOP")

  ;; Three 26-byte v0 AddShape records for cell "TOP":
  ;;   record A at offset 16, layer 1  (y1 field at 16 + 22 = 38)
  ;;   record B at offset 48, layer 2  (y1 field at 48 + 22 = 70)
  ;;   record C at offset 80, layer 3  (y1 field at 80 + 22 = 102)
  ;; Each is: opcode 01 | name_len 3 | "TOP" | layer | datatype 0 | x0 y0 x1 y1 (all 0).
  (data (i32.const 16) "\01\03\00TOP\01\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00")
  (data (i32.const 48) "\01\03\00TOP\02\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00")
  (data (i32.const 80) "\01\03\00TOP\03\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00\00")

  (func (export "run")
    ;; record A.y1 = number of shapes in cell "TOP"
    (i32.store align=1 (i32.const 38) (call $query_shapes (i32.const 0) (i32.const 3)))
    (drop (call $stage_edit (i32.const 16) (i32.const 26)))
    ;; record B.y1 = resolved selection count
    (i32.store align=1 (i32.const 70) (call $query_selection))
    (drop (call $stage_edit (i32.const 48) (i32.const 26)))
    ;; record C.y1 = active technology dbu_per_micron
    (i32.store align=1 (i32.const 102) (call $query_technology))
    (drop (call $stage_edit (i32.const 80) (i32.const 26)))
  )
)
