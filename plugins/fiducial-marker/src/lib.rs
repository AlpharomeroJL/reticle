//! Reticle sample plugin `fiducial-marker` (ADR 0116, the v0 guest contract).
//!
//! A minimal, REAL guest WebAssembly module (not hand-authored `.wat`, and not faked): it
//! imports the two `reticle` host functions `manifest.json` grants permission for, exports
//! its linear memory as `memory` (the `wasm32-unknown-unknown` `cdylib` default; no linker
//! flag needed) and an entry function `run` with signature `() -> ()`, and does one small,
//! genuinely useful thing: it reads how many shapes already exist in cell `TOP` (via
//! `query_shapes`, gated on the `read_document` permission) and stages an `AddShape`
//! rectangle (via `stage_edit`, gated on `stage_edit`) -- a small square "fiducial" marker on
//! a reserved marker layer, sized so it visibly grows with the cell's existing shape count.
//! Real IC layouts commonly carry small fiducial/registration marks on a reserved layer for
//! alignment; this one additionally encodes current occupancy, so a glance at its size hints
//! at how busy the cell is.
//!
//! No dependencies at all: rather than pull `reticle-model`/`reticle-geometry` into a
//! `wasm32-unknown-unknown` guest, this hand-encodes the v0 `AddShape` wire record exactly as
//! ADR 0116 specifies it (little-endian, opcode `0x01`). `#![no_std]` with no `alloc`: the
//! record is a fixed-size array on the guest's own stack, so there is no heap and no global
//! allocator to configure. `panic = "abort"` (see `Cargo.toml`) sidesteps the `eh_personality`
//! lang item a `no_std` crate would otherwise need for the (unsupported, on wasm32) unwind
//! panic strategy; the panic handler itself traps via `core::arch::wasm32::unreachable`, so a
//! guest bug becomes a clean wasm trap (`HostError::Trap` at the host), never undefined
//! behavior crossing the host/guest boundary.
//!
//! Degrades gracefully rather than trapping: if `TOP` does not exist yet, `query_shapes`
//! returns a negative error code, which this guest folds to zero (the marker still lands, at
//! its base size). If `TOP` truly does not exist at apply time, the funneled `AddShape` fails
//! with `ModelError::CellNotFound` on the *host* side (recorded in `RunOutcome::apply_errors`,
//! never a panic); it is not this guest's job to fabricate the cell.
#![no_std]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    // Surfaces at the host as `HostError::Trap`, never a panic across the host/guest
    // boundary.
    core::arch::wasm32::unreachable()
}

// The v0 host-function imports this guest uses (ADR 0116's table), from the `reticle`
// module namespace. Only functions whose permission `manifest.json` grants are wired into
// the linker; an import outside the v0 table, or one the manifest didn't grant, is rejected
// before this module ever runs (`HostError::UnknownImport` / `HostError::PermissionDenied`).
#[link(wasm_import_module = "reticle")]
extern "C" {
    /// Shape count of the named cell (needs `read_document`); `-1` bad pointer, `-2` bad
    /// UTF-8, `-3` no such cell. This guest folds any negative result to zero rather than
    /// trapping, so it degrades gracefully if `TOP` does not exist yet.
    fn query_shapes(cell_ptr: i32, cell_len: i32) -> i32;
    /// Appends a decoded v0 edit record to the run's staging buffer (needs `stage_edit`);
    /// `0` ok, negative on a malformed record or a full staging buffer.
    fn stage_edit(ptr: i32, len: i32) -> i32;
}

/// The cell this sample annotates. Fixed for v0 simplicity: the ABI has no
/// plugin-configuration channel yet, so there is nowhere to read a target cell name from
/// other than a compiled-in constant.
const CELL_NAME: &[u8] = b"TOP";
/// A layer/datatype reserved for plugin markers, chosen well away from the low layer
/// numbers real design geometry uses in the test fixtures and demos.
const MARKER_LAYER: u16 = 900;
const MARKER_DATATYPE: u16 = 0;
/// Marker half-extent before the occupancy term; keeps the marker visible even when `TOP`
/// is empty.
const BASE_SIZE: i32 = 4;

/// Byte length of the v0 `AddShape` record for a fixed 3-byte cell name (ADR 0116):
/// `opcode(1) + name_len(2) + name(3) + layer(2) + datatype(2) + x0/y0/x1/y1 (4 each)`.
const RECORD_LEN: usize = 1 + 2 + 3 + 2 + 2 + 4 * 4;

/// The plugin's entry point (named `run`, matching `manifest.json`'s `entry`), signature
/// `() -> ()` as ADR 0116 requires.
#[no_mangle]
pub extern "C" fn run() {
    // SAFETY: `query_shapes` is a host import wired in because `manifest.json` grants
    // `read_document`; `CELL_NAME` is `'static` and its length fits comfortably in `i32`, so
    // the pointer/length pair stays valid for the duration of this call.
    let count = unsafe { query_shapes(CELL_NAME.as_ptr() as i32, CELL_NAME.len() as i32) };
    let existing = if count >= 0 { count } else { 0 };
    let size = BASE_SIZE.saturating_add(existing);

    let record = build_add_shape_record(size);

    // SAFETY: `record` lives on this call's own stack and outlives the call; `stage_edit`
    // is wired in because `manifest.json` grants `stage_edit`.
    let _ = unsafe { stage_edit(record.as_ptr() as i32, record.len() as i32) };
}

/// Builds the v0 `AddShape` wire record (ADR 0116, little-endian) for a `size` x `size`
/// square rooted at the origin, on [`CELL_NAME`] / [`MARKER_LAYER`] / [`MARKER_DATATYPE`]:
///
/// ```text
/// u8  opcode (0x01)
/// u16 cell_name_len
/// u8  cell_name[cell_name_len]
/// u16 layer, u16 datatype
/// i32 x0, y0, x1, y1
/// ```
fn build_add_shape_record(size: i32) -> [u8; RECORD_LEN] {
    let mut record = [0u8; RECORD_LEN];
    let mut pos = 0usize;

    record[pos] = 0x01; // AddShape
    pos += 1;

    let name_len = CELL_NAME.len();
    record[pos..pos + 2].copy_from_slice(&(name_len as u16).to_le_bytes());
    pos += 2;
    record[pos..pos + name_len].copy_from_slice(CELL_NAME);
    pos += name_len;

    record[pos..pos + 2].copy_from_slice(&MARKER_LAYER.to_le_bytes());
    pos += 2;
    record[pos..pos + 2].copy_from_slice(&MARKER_DATATYPE.to_le_bytes());
    pos += 2;

    record[pos..pos + 4].copy_from_slice(&0i32.to_le_bytes()); // x0
    pos += 4;
    record[pos..pos + 4].copy_from_slice(&0i32.to_le_bytes()); // y0
    pos += 4;
    record[pos..pos + 4].copy_from_slice(&size.to_le_bytes()); // x1
    pos += 4;
    record[pos..pos + 4].copy_from_slice(&size.to_le_bytes()); // y1
    pos += 4;

    debug_assert_eq!(
        pos, RECORD_LEN,
        "record layout must fill the buffer exactly"
    );
    record
}
