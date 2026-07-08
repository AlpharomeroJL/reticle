# KLayout batch generator for crates/reticle-drc/tests/fixtures/subset.gds.
#
# Emits the exact geometry that tests/lydrc_engine.rs reconstructs as a Reticle
# Document, so the two DRC engines see the identical layout. Coordinates are DBU
# (dbu = 0.001 um = 1 nm), matching the Reticle convention (1 dbu = 1 nm). Keep
# this in lock-step with subset_layout() in tests/lydrc_engine.rs.
#
# Run headless (inside the pinned container, see scripts/lydrc-compare.ps1):
#   klayout -b -r scripts/lydrc-fixture-gen.rb -rd out=/work/subset.gds
#
# Cell "SUBSET" holds seven boxes so each supported subset rule fires exactly
# once, except the clean met2 pad:
#   * met1 narrow wire      -> width  (m1.1)
#   * met1 close pair       -> space  (m1.2)
#   * met1 pad over mcon    -> encl.  (m1.4)  (10 dbu margin < 30 required)
#   * li1 small pad         -> area   (li.6)  (40_000 dbu2 < 56_100 required)
#   * met2 wide/large pad   -> width  (m2.1)  clean, fires nothing

out = $out || "subset.gds"

ly = RBA::Layout.new
ly.dbu = 0.001
top = ly.create_cell("SUBSET")

met1 = ly.layer(68, 20)
met2 = ly.layer(69, 20)
mcon = ly.layer(67, 44)
li1  = ly.layer(67, 20)

# met1: narrow wire, 100 dbu wide (< 140) -> m1.1.
top.shapes(met1).insert(RBA::Box.new(0, 0, 1000, 100))
# met1: a close pair, 100 dbu apart (< 140) -> m1.2. Each 200x200 so width is fine.
top.shapes(met1).insert(RBA::Box.new(0, 10_000, 200, 10_200))
top.shapes(met1).insert(RBA::Box.new(300, 10_000, 500, 10_200))
# met1: 220x220 pad enclosing the mcon cut by only 10 dbu (< 30) -> m1.4.
top.shapes(met1).insert(RBA::Box.new(9_990, -10, 10_210, 210))
# mcon: 200x200 cut enclosed by the pad above.
top.shapes(mcon).insert(RBA::Box.new(10_000, 0, 10_200, 200))
# li1: 200x200 pad, area 40_000 dbu2 (< 56_100) -> li.6.
top.shapes(li1).insert(RBA::Box.new(0, 20_000, 200, 20_200))
# met2: 500x500 pad, wide and large -> m2.1 clean.
top.shapes(met2).insert(RBA::Box.new(0, 30_000, 500, 30_500))

ly.write(out)
puts "lydrc-fixture-gen: wrote #{out} (dbu=#{ly.dbu})"
