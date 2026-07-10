//! DXF 2D reader: the subset relevant to layout import (LWPOLYLINE, POLYLINE,
//! LINE, HATCH boundary loops, and CIRCLE/ARC polygonized at a documented
//! tolerance), returning the layer mapping as data for the UI to resolve, with
//! every count-driven allocation capped against the remaining input.
//!
//! Reserved by the Phase 1 pre-fan-out commit and implemented by the `dxf` lane.
//! Until then this module is intentionally empty so the workspace and `just ci`
//! stay green and no lane has to touch `lib.rs`.
