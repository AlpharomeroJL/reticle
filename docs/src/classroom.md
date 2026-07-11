# Classroom mode

Scaffolded in the v8.2 campaign Phase 3. Classroom mode layers a teaching
workflow on top of the multi-writer collaboration relay: an instructor can bring
every student's view to the current cell, students can follow the instructor's
viewport, and the instructor can unlock an individual student to work
independently.

It reuses the existing presence and relay machinery (crate `reticle-sync`); the
`share.rs` server default stays `127.0.0.1:3030` (the public-relay story remains
operator-owned, tracked as H1). This chapter is filled by the `classroom` lane.
