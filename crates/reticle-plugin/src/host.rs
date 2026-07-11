//! The native embedded WebAssembly plugin host: a `wasmi` interpreter that runs
//! an untrusted plugin under fuel and linear-memory limits, exposes the v0
//! host-function table, gates every host function against the manifest
//! permissions at instantiation, and funnels every plugin edit through the
//! command and undo machinery so a plugin's whole effect is undoable and
//! replayable by construction.
//!
//! Native-only (`cfg(not(target_arch = "wasm32"))`, ADR 0115): the interpreter
//! never enters the wasm bundle. The browser build offers browse/preview and a
//! "plugins run in the desktop app" disclaimer, the same shape as the native-only
//! rhai producer and the real agent.
//!
//! # v0 calling convention (proven here, implemented in full by plugin-host)
//!
//! A plugin is a WebAssembly module in **binary** form. It exports its linear
//! memory as `"memory"` and an entry function named by [`Manifest::entry`] with
//! signature `() -> ()`. It imports host functions from the module namespace
//! [`HOST_MODULE`]; only the functions whose [`Permission`] the manifest grants
//! are wired in, so importing an ungranted (or unknown) host function is rejected
//! at instantiation.
//!
//! The host-function table (all pointers/lengths are `i32` indices into the
//! plugin's exported memory; every guest-supplied region is bounds-checked, and
//! an invalid region yields a negative error code, never a host panic or an
//! out-of-bounds read):
//!
//! - `query_shapes(cell_ptr, cell_len) -> i32` ([`Permission::ReadDocument`]):
//!   the shape count of the named cell, or `-1` bad pointer / `-2` bad UTF-8 /
//!   `-3` cell not found.
//! - `query_selection() -> i32` ([`Permission::ReadSelection`]): the number of
//!   selected shapes that resolve to a real shape in the pre-run document snapshot.
//! - `query_technology() -> i32` ([`Permission::ReadTechnology`]): the active
//!   technology's `dbu_per_micron`.
//! - `stage_edit(ptr, len) -> i32` ([`Permission::StageEdit`]): decodes a v0 edit
//!   record from `[ptr, ptr+len)` and appends it to the staging buffer; `0` on
//!   success, `-1` bad pointer / `-2` malformed record / `-3` staging buffer full.
//!
//! Staged edits are **not** applied during the run. After the entry returns, the
//! host replays them onto the caller's [`EditableDocument`] through
//! [`EditableDocument::apply`] (the funnel), so they land as one contiguous run of
//! undo-stack entries.
//!
//! # v0 edit wire format (the `stage_edit` payload, little-endian)
//!
//! Untrusted guest bytes; every count is capped against the remaining byte budget
//! and a short or malformed record errors ([`EditDecodeError`]) rather than
//! panicking. Two shared sub-encodings recur:
//!
//! ```text
//! name       := u16 len   (capped at Limits::max_query_len) ++ len bytes UTF-8
//! transform  := u8 orientation (0..8, per Orientation::code)
//!               i32 tx, i32 ty                (translation, DBU)
//!               u32 mag_num, u32 mag_den      (magnification; den != 0)
//! ```
//!
//! The opcode byte selects the record; every [`Edit`] variant has one:
//!
//! ```text
//! u8 opcode
//! 0x01 AddShape     : name cell, u16 layer, u16 datatype, i32 x0,y0,x1,y1
//! 0x02 AddCell      : name cell                         (adds an empty cell)
//! 0x03 RemoveCell   : name cell
//! 0x04 RemoveShape  : name cell, u32 index
//! 0x05 AddInstance  : name cell, name child, transform
//! 0x06 AddArray     : name cell, name child, transform,
//!                     u32 columns, u32 rows, i32 column_pitch, i32 row_pitch
//! 0x07 AddLabel     : name cell, name text, i32 x, i32 y,
//!                     u16 layer, u16 datatype, u8 anchor (0..5, per Anchor)
//! 0x08 RemoveLabel  : name cell, u32 index
//! ```
//!
//! The spike pinned `AddShape` (ADR 0116); plugin-host adds the rest of the
//! [`Edit`] vocabulary against the same framing and `Cursor` cap discipline
//! (ADR 0117). An index or count field is decoded raw and validated by the funnel
//! ([`EditableDocument::apply`] bounds-checks it), so a decoded record is always a
//! well-formed [`Edit`] even when it will not apply to a given document.

use crate::manifest::{HostFn, Manifest, ManifestError, Permission};
use reticle_geometry::{LayerId, Magnification, Orientation, Point, Rect, Transform};
use reticle_model::{
    Anchor, ArrayInstance, Cell, Document, DocumentStore, DrawShape, Edit, EditableDocument,
    Instance, Label, ModelError, ShapeKind,
};
use std::fmt;
use wasmi::{
    AsContext, Caller, Config, Engine, ExternType, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder,
};

/// The import module namespace every host function lives under.
pub const HOST_MODULE: &str = "reticle";
/// The linear-memory export name a plugin must expose to use memory-passing host
/// functions.
pub const HOST_MEMORY_EXPORT: &str = "memory";

/// v0 edit opcodes: one per [`Edit`] variant. `AddShape` (`0x01`) was pinned by
/// the spike (ADR 0116); the rest are pinned by ADR 0117 on the same framing.
const OP_ADD_SHAPE: u8 = 0x01;
/// Opcode: add an empty cell by name.
const OP_ADD_CELL: u8 = 0x02;
/// Opcode: remove a cell by name.
const OP_REMOVE_CELL: u8 = 0x03;
/// Opcode: remove the shape at an index from a cell.
const OP_REMOVE_SHAPE: u8 = 0x04;
/// Opcode: add a single instance placement to a cell.
const OP_ADD_INSTANCE: u8 = 0x05;
/// Opcode: add an array placement to a cell.
const OP_ADD_ARRAY: u8 = 0x06;
/// Opcode: append a label to a cell.
const OP_ADD_LABEL: u8 = 0x07;
/// Opcode: remove the label at an index from a cell.
const OP_REMOVE_LABEL: u8 = 0x08;

/// Default execution fuel: a generous ceiling for a small plugin; a runaway
/// plugin exhausts it and traps. Measured consumption is reported in
/// [`RunOutcome::fuel_consumed`].
pub const DEFAULT_FUEL: u64 = 10_000_000;
/// Default linear-memory ceiling (16 MiB): growth past it fails, trapping the
/// plugin rather than letting it exhaust host memory.
pub const DEFAULT_MEMORY_BYTES: usize = 16 * 1024 * 1024;
/// Default cap on the plugin binary size (4 MiB); a larger module is rejected
/// before compilation.
pub const DEFAULT_MAX_WASM_BYTES: usize = 4 * 1024 * 1024;
/// Default cap on the number of edits a single run may stage (bounds host-side
/// allocation from a hostile plugin).
pub const DEFAULT_MAX_STAGED_EDITS: usize = 100_000;
/// Default cap on a query/edit cell-name length in bytes.
pub const DEFAULT_MAX_QUERY_LEN: usize = 256;
/// Default cap on a single `stage_edit` payload in bytes.
pub const DEFAULT_MAX_EDIT_LEN: usize = 64 * 1024;

/// The resource ceilings a single plugin run executes under. All are enforced;
/// [`Limits::default`] gives sensible values.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Execution fuel; exhaustion traps the plugin (`wasmi` `OutOfFuel`).
    pub fuel: u64,
    /// Maximum bytes the plugin's linear memory may grow to.
    pub memory_bytes: usize,
    /// Maximum accepted plugin binary size in bytes.
    pub max_wasm_bytes: usize,
    /// Maximum edits a run may stage before `stage_edit` starts rejecting.
    pub max_staged_edits: usize,
    /// Maximum cell-name length a query or edit record may carry, in bytes.
    pub max_query_len: usize,
    /// Maximum `stage_edit` payload length in bytes.
    pub max_edit_len: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: DEFAULT_FUEL,
            memory_bytes: DEFAULT_MEMORY_BYTES,
            max_wasm_bytes: DEFAULT_MAX_WASM_BYTES,
            max_staged_edits: DEFAULT_MAX_STAGED_EDITS,
            max_query_len: DEFAULT_MAX_QUERY_LEN,
            max_edit_len: DEFAULT_MAX_EDIT_LEN,
        }
    }
}

/// A read-only reference to one selected shape: the cell that owns it and the
/// index of the shape within that cell's `shapes`. The host resolves each
/// reference against the pre-run document snapshot, so `query_selection` counts
/// only selections that name a real shape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SelectedShape {
    /// Name of the cell the selected shape lives in.
    pub cell: String,
    /// Index into that cell's `shapes`.
    pub index: usize,
}

impl SelectedShape {
    /// A reference to the shape at `index` in cell `cell`.
    pub fn new(cell: impl Into<String>, index: usize) -> Self {
        Self {
            cell: cell.into(),
            index,
        }
    }
}

/// The read-only inputs a run exposes to the plugin's query host functions, over
/// and above the document snapshot (which the host takes from the
/// [`EditableDocument`] passed to [`Host::run`]).
///
/// The selection is passed as real shape references rather than a precomputed
/// count so `query_selection` answers from actual document state: the host
/// resolves each reference against the pre-run snapshot and returns the number
/// that name a shape that exists, so a stale reference cannot inflate the answer
/// and the count is reproducible across identical runs.
#[derive(Clone, Debug, Default)]
pub struct HostContext {
    /// The shapes selected when the run begins. `query_selection` returns how many
    /// of these resolve to a real shape in the pre-run document snapshot.
    pub selection: Vec<SelectedShape>,
}

/// The outcome of a plugin run: the edits it staged, how many the funnel applied,
/// any that failed to apply, and the fuel it consumed.
#[derive(Debug)]
pub struct RunOutcome {
    /// Every edit the plugin staged, in call order (for inspection and replay).
    pub staged: Vec<Edit>,
    /// How many staged edits the funnel applied through [`EditableDocument::apply`].
    pub applied: usize,
    /// Staged edits that failed to apply (for example a missing target cell); the
    /// run does not abort on these, it records and skips them.
    pub apply_errors: Vec<ModelError>,
    /// Fuel consumed by the run (`limits.fuel` minus the remaining fuel).
    pub fuel_consumed: u64,
}

/// A failure to load, gate, instantiate, or run a plugin. Every path that touches
/// untrusted bytes yields one of these rather than panicking.
#[derive(Debug)]
pub enum HostError {
    /// The manifest itself did not validate.
    Manifest(ManifestError),
    /// The plugin binary exceeded [`Limits::max_wasm_bytes`].
    TooLarge {
        /// The observed binary length.
        len: usize,
        /// The configured cap.
        cap: usize,
    },
    /// The plugin bytes did not compile (malformed, unsupported, or text-format).
    Compile(String),
    /// The plugin imported a name from [`HOST_MODULE`] that is not a v0 host
    /// function, or imported it as something other than a function.
    UnknownImport {
        /// The offending import field name.
        name: String,
    },
    /// The plugin imported a host function whose [`Permission`] the manifest did
    /// not grant; rejected before instantiation.
    PermissionDenied {
        /// The host function that was denied.
        host_fn: HostFn,
        /// The permission it required.
        permission: Permission,
    },
    /// Configuring the store (for example setting fuel) failed.
    Config(String),
    /// Wiring a host function into the linker failed.
    Link(String),
    /// Instantiation failed (an unresolved import, a type mismatch, or a trapping
    /// start/limiter, including linear memory over the cap).
    Instantiate(String),
    /// The module did not export the manifest's entry function with signature
    /// `() -> ()`.
    MissingEntry {
        /// The entry export name that was expected.
        name: String,
    },
    /// The plugin trapped while running (unreachable, out of fuel, out of memory,
    /// or an explicit host trap).
    Trap(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(e) => write!(f, "manifest did not validate: {e}"),
            Self::TooLarge { len, cap } => {
                write!(f, "plugin binary is {len} bytes, over the {cap} cap")
            }
            Self::Compile(e) => write!(f, "plugin did not compile: {e}"),
            Self::UnknownImport { name } => {
                write!(
                    f,
                    "plugin imports unknown host function `{HOST_MODULE}::{name}`"
                )
            }
            Self::PermissionDenied {
                host_fn,
                permission,
            } => write!(
                f,
                "plugin imports `{host_fn:?}` but the manifest does not grant `{permission:?}`"
            ),
            Self::Config(e) => write!(f, "store configuration failed: {e}"),
            Self::Link(e) => write!(f, "linking a host function failed: {e}"),
            Self::Instantiate(e) => write!(f, "instantiation failed: {e}"),
            Self::MissingEntry { name } => {
                write!(f, "plugin does not export entry `{name}` as `() -> ()`")
            }
            Self::Trap(e) => write!(f, "plugin trapped: {e}"),
        }
    }
}

impl std::error::Error for HostError {}

/// A failure to decode a v0 `stage_edit` record from untrusted guest bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditDecodeError {
    /// The record ended before a field could be read in full.
    UnexpectedEof {
        /// Bytes the field needed.
        needed: usize,
        /// Bytes actually remaining.
        remaining: usize,
    },
    /// The opcode byte is not a known v0 edit opcode.
    UnknownOpcode(u8),
    /// The declared cell-name length exceeded the cap.
    NameTooLong {
        /// The declared length.
        len: usize,
        /// The configured cap.
        cap: usize,
    },
    /// The cell name was not valid UTF-8.
    BadUtf8,
    /// A transform orientation code was not one of the eight valid values (`0..8`).
    BadOrientation(u8),
    /// A transform magnification record had a zero denominator (an undefined ratio).
    ZeroMagDenominator,
    /// A label anchor code was not one of the valid anchors (`0..5`).
    BadAnchor(u8),
}

impl fmt::Display for EditDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { needed, remaining } => write!(
                f,
                "edit record truncated: needed {needed} more bytes, {remaining} remain"
            ),
            Self::UnknownOpcode(op) => write!(f, "unknown v0 edit opcode {op:#04x}"),
            Self::NameTooLong { len, cap } => {
                write!(f, "edit cell name is {len} bytes, over the {cap} cap")
            }
            Self::BadUtf8 => write!(f, "edit cell name is not valid UTF-8"),
            Self::BadOrientation(code) => {
                write!(f, "edit transform orientation code {code} is not in 0..8")
            }
            Self::ZeroMagDenominator => {
                write!(f, "edit transform magnification denominator is zero")
            }
            Self::BadAnchor(code) => write!(f, "edit label anchor code {code} is not in 0..5"),
        }
    }
}

impl std::error::Error for EditDecodeError {}

/// The per-run host state carried by the `wasmi` [`Store`]: the read-only query
/// snapshot, the caps, the staging buffer, and the linear-memory limiter.
struct HostState {
    /// Document snapshot taken before the run, so queries are reproducible
    /// regardless of what the plugin stages.
    doc: Document,
    /// The selection count returned by `query_selection`.
    selection_count: i32,
    /// The technology resolution returned by `query_technology`.
    dbu_per_micron: i64,
    /// Edits the plugin has staged so far this run.
    staged: Vec<Edit>,
    /// Cap on [`HostState::staged`].
    max_staged: usize,
    /// Cap on a query/edit cell-name length.
    max_query_len: usize,
    /// Cap on a `stage_edit` payload length.
    max_edit_len: usize,
    /// Linear-memory limiter, consulted by the interpreter on every memory grow.
    limits: StoreLimits,
}

/// The native embedded WebAssembly plugin host. One [`Host`] owns a fuel-metering
/// [`Engine`] and can run many plugins.
#[derive(Debug)]
pub struct Host {
    /// The fuel-metering interpreter engine.
    engine: Engine,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    /// Creates a host whose engine meters fuel (so [`Limits::fuel`] is enforced).
    #[must_use]
    pub fn new() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        Self {
            engine: Engine::new(&config),
        }
    }

    /// Loads, gates, instantiates, and runs `wasm` against `doc` under `limits`,
    /// funneling every staged edit through [`EditableDocument::apply`].
    ///
    /// Determinism: the query snapshot is cloned from `doc` before any edit is
    /// applied, so re-running the same plugin bytes and manifest against an equal
    /// document reproduces the same staged edits and the same resulting document.
    ///
    /// # Errors
    ///
    /// Returns a [`HostError`] if the manifest is invalid, the binary is oversized
    /// or malformed, the plugin imports an ungranted or unknown host function, the
    /// module lacks the entry export, or the plugin traps (including exhausting
    /// fuel or growing memory past the cap). Never panics on plugin-controlled
    /// input.
    pub fn run(
        &self,
        wasm: &[u8],
        manifest: &Manifest,
        doc: &mut EditableDocument,
        ctx: &HostContext,
        limits: &Limits,
    ) -> Result<RunOutcome, HostError> {
        manifest.validate().map_err(HostError::Manifest)?;
        if wasm.len() > limits.max_wasm_bytes {
            return Err(HostError::TooLarge {
                len: wasm.len(),
                cap: limits.max_wasm_bytes,
            });
        }

        // Binary-only: the `wat` feature is off, so a text-format module fails here.
        let module =
            Module::new(&self.engine, wasm).map_err(|e| HostError::Compile(e.to_string()))?;

        let granted = manifest.permissions.as_slice();
        scan_imports(&module, granted)?;

        // Read-only snapshot BEFORE any apply, so queries never observe the
        // plugin's own in-progress edits (this is what makes runs reproducible).
        let snapshot = doc.document().clone();
        let dbu_per_micron = snapshot.technology().dbu_per_micron;
        // Resolve the selection against the snapshot so `query_selection` reflects
        // real document state: a reference to a missing cell or an out-of-range shape
        // index is dropped rather than counted, and the answer is reproducible from
        // the pre-run snapshot exactly like the other read-only queries.
        let selection_count = i32::try_from(
            ctx.selection
                .iter()
                .filter(|s| match snapshot.cell(&s.cell) {
                    Some(cell) => s.index < cell.shapes.len(),
                    None => false,
                })
                .count(),
        )
        .unwrap_or(i32::MAX);
        let state = HostState {
            doc: snapshot,
            selection_count,
            dbu_per_micron,
            staged: Vec::new(),
            max_staged: limits.max_staged_edits,
            max_query_len: limits.max_query_len,
            max_edit_len: limits.max_edit_len,
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .build(),
        };

        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits as &mut dyn wasmi::ResourceLimiter);
        store
            .set_fuel(limits.fuel)
            .map_err(|e| HostError::Config(e.to_string()))?;

        let mut linker = Linker::new(&self.engine);
        wire_host_fns(&mut linker, granted)?;

        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| HostError::Instantiate(e.to_string()))?;

        let entry = instance
            .get_typed_func::<(), ()>(&store, &manifest.entry)
            .map_err(|_| HostError::MissingEntry {
                name: manifest.entry.clone(),
            })?;

        entry
            .call(&mut store, ())
            .map_err(|e| HostError::Trap(e.to_string()))?;

        let fuel_after = store.get_fuel().unwrap_or(0);
        let fuel_consumed = limits.fuel.saturating_sub(fuel_after);

        let staged = std::mem::take(&mut store.data_mut().staged);

        // The funnel: staged edits reach the document ONLY through
        // EditableDocument::apply, so the plugin's effect is one contiguous,
        // undoable run of undo-stack entries.
        let mut applied = 0usize;
        let mut apply_errors = Vec::new();
        for edit in &staged {
            match doc.apply(edit.clone()) {
                Ok(()) => applied += 1,
                Err(e) => apply_errors.push(e),
            }
        }

        Ok(RunOutcome {
            staged,
            applied,
            apply_errors,
            fuel_consumed,
        })
    }
}

/// Maps a [`HOST_MODULE`] import field name to the v0 [`HostFn`] it names.
fn host_fn_from_name(name: &str) -> Option<HostFn> {
    match name {
        "query_shapes" => Some(HostFn::QueryShapes),
        "query_selection" => Some(HostFn::QuerySelection),
        "query_technology" => Some(HostFn::QueryTechnology),
        "stage_edit" => Some(HostFn::StageEdit),
        _ => None,
    }
}

/// Rejects, before instantiation, any [`HOST_MODULE`] import that is unknown, is
/// not a function, or names a host function whose permission was not granted.
/// Imports from other namespaces are left for the linker to resolve (and fail).
fn scan_imports(module: &Module, granted: &[Permission]) -> Result<(), HostError> {
    for import in module.imports() {
        if import.module() != HOST_MODULE {
            continue;
        }
        let name = import.name();
        let Some(host_fn) = host_fn_from_name(name) else {
            return Err(HostError::UnknownImport {
                name: name.to_owned(),
            });
        };
        if !matches!(import.ty(), ExternType::Func(_)) {
            return Err(HostError::UnknownImport {
                name: name.to_owned(),
            });
        }
        let permission = host_fn.required_permission();
        if !granted.contains(&permission) {
            return Err(HostError::PermissionDenied {
                host_fn,
                permission,
            });
        }
    }
    Ok(())
}

/// Wires exactly the host functions whose permission the manifest granted into the
/// linker. Because [`scan_imports`] has already rejected any ungranted import,
/// this covers every function the module can import.
fn wire_host_fns(linker: &mut Linker<HostState>, granted: &[Permission]) -> Result<(), HostError> {
    if granted.contains(&Permission::ReadDocument) {
        linker
            .func_wrap(
                HOST_MODULE,
                "query_shapes",
                |caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
                    let cap = caller.data().max_query_len;
                    let Some(name_bytes) = read_guest_bytes(&caller, ptr, len, cap) else {
                        return -1;
                    };
                    let Ok(name) = std::str::from_utf8(&name_bytes) else {
                        return -2;
                    };
                    match caller.data().doc.cell(name) {
                        Some(cell) => i32::try_from(cell.shapes.len()).unwrap_or(i32::MAX),
                        None => -3,
                    }
                },
            )
            .map_err(|e| HostError::Link(e.to_string()))?;
    }

    if granted.contains(&Permission::ReadSelection) {
        linker
            .func_wrap(
                HOST_MODULE,
                "query_selection",
                |caller: Caller<'_, HostState>| -> i32 { caller.data().selection_count },
            )
            .map_err(|e| HostError::Link(e.to_string()))?;
    }

    if granted.contains(&Permission::ReadTechnology) {
        linker
            .func_wrap(
                HOST_MODULE,
                "query_technology",
                |caller: Caller<'_, HostState>| -> i32 {
                    i32::try_from(caller.data().dbu_per_micron).unwrap_or(-1)
                },
            )
            .map_err(|e| HostError::Link(e.to_string()))?;
    }

    if granted.contains(&Permission::StageEdit) {
        linker
            .func_wrap(
                HOST_MODULE,
                "stage_edit",
                |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
                    let (edit_cap, name_cap, staged_cap, staged_len) = {
                        let s = caller.data();
                        (
                            s.max_edit_len,
                            s.max_query_len,
                            s.max_staged,
                            s.staged.len(),
                        )
                    };
                    if staged_len >= staged_cap {
                        return -3;
                    }
                    let Some(bytes) = read_guest_bytes(&caller, ptr, len, edit_cap) else {
                        return -1;
                    };
                    let Ok(edit) = decode_edit_v0(&bytes, name_cap) else {
                        return -2;
                    };
                    caller.data_mut().staged.push(edit);
                    0
                },
            )
            .map_err(|e| HostError::Link(e.to_string()))?;
    }

    Ok(())
}

/// Copies `len` bytes from the plugin's exported memory at `ptr`, or returns
/// `None` for a negative/oversized/out-of-bounds region or a missing memory
/// export. Every guest-supplied region flows through here, so a hostile pointer
/// is a clean `None`, never a host-side out-of-bounds access.
fn read_guest_bytes(
    caller: &Caller<'_, HostState>,
    ptr: i32,
    len: i32,
    cap: usize,
) -> Option<Vec<u8>> {
    if ptr < 0 || len < 0 {
        return None;
    }
    let len = len as usize;
    if len > cap {
        return None;
    }
    let memory = caller.get_export(HOST_MEMORY_EXPORT)?.into_memory()?;
    let mut buf = vec![0u8; len];
    // `Memory::read` bounds-checks `ptr + len` against the current memory size and
    // errors (no panic) on overflow, which we map to `None`.
    memory
        .read(caller.as_context(), ptr as usize, &mut buf)
        .ok()?;
    Some(buf)
}

/// Decodes a v0 edit record (see the module docs) from untrusted guest bytes into
/// the [`Edit`] it names.
///
/// Every length is capped both against the remaining byte budget (through
/// `Cursor`) and against `max_name`, so a truncated, oversized, or otherwise
/// hostile record returns a structured [`EditDecodeError`] rather than over-reading,
/// over-allocating, or panicking. This is the one decoder behind the `stage_edit`
/// host function and the target of the `plugin_decode_edit` cargo-fuzz target.
///
/// A decoded record is always a well-formed [`Edit`], but not necessarily one that
/// applies to a given document: an index or a target cell is validated by the funnel
/// ([`EditableDocument::apply`]), which reports a [`ModelError`] the run records and
/// skips rather than aborting on.
///
/// # Errors
///
/// Returns an [`EditDecodeError`] for an unknown opcode, a truncated record, a name
/// over `max_name` or not valid UTF-8, or an out-of-range orientation, a zero
/// magnification denominator, or an unknown anchor.
pub fn decode_edit_v0(bytes: &[u8], max_name: usize) -> Result<Edit, EditDecodeError> {
    let mut c = Cursor::new(bytes);
    let opcode = c.u8()?;
    match opcode {
        OP_ADD_SHAPE => {
            let cell = read_name(&mut c, max_name)?;
            let layer = c.u16()?;
            let datatype = c.u16()?;
            let x0 = c.i32()?;
            let y0 = c.i32()?;
            let x1 = c.i32()?;
            let y1 = c.i32()?;
            let rect = Rect::new(Point::new(x0, y0), Point::new(x1, y1));
            let shape = DrawShape::new(LayerId::new(layer, datatype), ShapeKind::Rect(rect));
            Ok(Edit::AddShape { cell, shape })
        }
        OP_ADD_CELL => {
            let name = read_name(&mut c, max_name)?;
            Ok(Edit::AddCell {
                cell: Cell::new(name),
            })
        }
        OP_REMOVE_CELL => {
            let name = read_name(&mut c, max_name)?;
            Ok(Edit::RemoveCell { name })
        }
        OP_REMOVE_SHAPE => {
            let cell = read_name(&mut c, max_name)?;
            let index = c.u32()? as usize;
            Ok(Edit::RemoveShape { cell, index })
        }
        OP_ADD_INSTANCE => {
            let cell = read_name(&mut c, max_name)?;
            let child = read_name(&mut c, max_name)?;
            let transform = decode_transform(&mut c)?;
            Ok(Edit::AddInstance {
                cell,
                instance: Instance {
                    cell: child,
                    transform,
                },
            })
        }
        OP_ADD_ARRAY => {
            let cell = read_name(&mut c, max_name)?;
            let child = read_name(&mut c, max_name)?;
            let transform = decode_transform(&mut c)?;
            let columns = c.u32()?;
            let rows = c.u32()?;
            let column_pitch = c.i32()?;
            let row_pitch = c.i32()?;
            Ok(Edit::AddArray {
                cell,
                array: ArrayInstance {
                    cell: child,
                    transform,
                    columns,
                    rows,
                    column_pitch,
                    row_pitch,
                },
            })
        }
        OP_ADD_LABEL => {
            let cell = read_name(&mut c, max_name)?;
            let text = read_name(&mut c, max_name)?;
            let x = c.i32()?;
            let y = c.i32()?;
            let layer = c.u16()?;
            let datatype = c.u16()?;
            let anchor = decode_anchor(c.u8()?)?;
            Ok(Edit::AddLabel {
                cell,
                label: Label {
                    text,
                    position: Point::new(x, y),
                    layer: LayerId::new(layer, datatype),
                    anchor,
                },
            })
        }
        OP_REMOVE_LABEL => {
            let cell = read_name(&mut c, max_name)?;
            let index = c.u32()? as usize;
            Ok(Edit::RemoveLabel { cell, index })
        }
        other => Err(EditDecodeError::UnknownOpcode(other)),
    }
}

/// Reads a length-prefixed name: a `u16` length capped at `max_name` and against
/// the bytes that remain, then that many bytes decoded as UTF-8.
fn read_name(c: &mut Cursor, max_name: usize) -> Result<String, EditDecodeError> {
    let len = c.u16()? as usize;
    if len > max_name {
        return Err(EditDecodeError::NameTooLong { len, cap: max_name });
    }
    let bytes = c.take(len)?;
    let name = std::str::from_utf8(bytes)
        .map_err(|_| EditDecodeError::BadUtf8)?
        .to_owned();
    Ok(name)
}

/// Decodes a placement [`Transform`]: a `u8` orientation code (`0..8`), an `i32`
/// translation, and a `u32 / u32` magnification whose denominator must be non-zero.
fn decode_transform(c: &mut Cursor) -> Result<Transform, EditDecodeError> {
    let code = c.u8()?;
    if usize::from(code) >= Orientation::ALL.len() {
        return Err(EditDecodeError::BadOrientation(code));
    }
    let orientation = Orientation::from_code(u32::from(code));
    let tx = c.i32()?;
    let ty = c.i32()?;
    let num = c.u32()?;
    let den = c.u32()?;
    let magnification = Magnification::new(num, den).ok_or(EditDecodeError::ZeroMagDenominator)?;
    Ok(Transform {
        translation: Point::new(tx, ty),
        orientation,
        magnification,
    })
}

/// Maps a `u8` anchor code to its [`Anchor`], matching `Anchor`'s declaration order.
fn decode_anchor(code: u8) -> Result<Anchor, EditDecodeError> {
    match code {
        0 => Ok(Anchor::Center),
        1 => Ok(Anchor::SouthWest),
        2 => Ok(Anchor::SouthEast),
        3 => Ok(Anchor::NorthWest),
        4 => Ok(Anchor::NorthEast),
        other => Err(EditDecodeError::BadAnchor(other)),
    }
}

/// A forward-only reader over a byte slice that caps every read against the bytes
/// that remain, so decoding untrusted input can never index out of bounds.
struct Cursor<'a> {
    /// The bytes being read.
    bytes: &'a [u8],
    /// The next unread offset; the invariant `pos <= bytes.len()` always holds.
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Wraps a slice at offset zero.
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Bytes not yet consumed.
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// Consumes and returns the next `n` bytes, or errors if fewer remain.
    fn take(&mut self, n: usize) -> Result<&'a [u8], EditDecodeError> {
        if n > self.remaining() {
            return Err(EditDecodeError::UnexpectedEof {
                needed: n,
                remaining: self.remaining(),
            });
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    /// Reads one byte.
    fn u8(&mut self) -> Result<u8, EditDecodeError> {
        Ok(self.take(1)?[0])
    }

    /// Reads a little-endian `u16`.
    fn u16(&mut self) -> Result<u16, EditDecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// Reads a little-endian `u32`.
    fn u32(&mut self) -> Result<u32, EditDecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads a little-endian `i32`.
    fn i32(&mut self) -> Result<i32, EditDecodeError> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed `AddShape` record decodes to the expected edit.
    #[test]
    fn decodes_a_well_formed_add_shape() {
        // opcode, name_len=3, "TOP", layer=1, datatype=0, x0=0 y0=0 x1=100 y1=200
        let mut bytes = vec![OP_ADD_SHAPE, 3, 0, b'T', b'O', b'P', 1, 0, 0, 0];
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&100i32.to_le_bytes());
        bytes.extend_from_slice(&200i32.to_le_bytes());
        let edit = decode_edit_v0(&bytes, 256).expect("valid record decodes");
        match edit {
            Edit::AddShape { cell, shape } => {
                assert_eq!(cell, "TOP");
                assert_eq!(shape.layer, LayerId::new(1, 0));
                match shape.kind {
                    ShapeKind::Rect(r) => {
                        assert_eq!(r.min, Point::new(0, 0));
                        assert_eq!(r.max, Point::new(100, 200));
                    }
                    _ => panic!("expected a rect"),
                }
            }
            _ => panic!("expected AddShape"),
        }
    }

    /// An unknown opcode is a clean error, not a panic.
    #[test]
    fn rejects_unknown_opcode() {
        assert_eq!(
            decode_edit_v0(&[0xFF], 256).unwrap_err(),
            EditDecodeError::UnknownOpcode(0xFF)
        );
    }

    /// A name length past the cap is a clean error.
    #[test]
    fn rejects_overlong_name() {
        // opcode, name_len = 5000 (0x1388) little-endian
        let bytes = [OP_ADD_SHAPE, 0x88, 0x13];
        assert!(matches!(
            decode_edit_v0(&bytes, 256),
            Err(EditDecodeError::NameTooLong {
                len: 5000,
                cap: 256
            })
        ));
    }

    /// Every truncation of a valid record errors rather than panicking: the
    /// count-against-remaining discipline holds for all prefixes (an in-process
    /// stand-in for the fuzz target plugin-host will add).
    #[test]
    fn every_truncation_errors_without_panicking() {
        let mut full = vec![OP_ADD_SHAPE, 3, 0, b'T', b'O', b'P', 1, 0, 0, 0];
        full.extend_from_slice(&1i32.to_le_bytes());
        full.extend_from_slice(&2i32.to_le_bytes());
        full.extend_from_slice(&3i32.to_le_bytes());
        full.extend_from_slice(&4i32.to_le_bytes());
        // Every strict prefix is incomplete and must Err (never panic, never Ok).
        for n in 0..full.len() {
            assert!(
                decode_edit_v0(&full[..n], 256).is_err(),
                "prefix of length {n} should not decode"
            );
        }
        // The full record decodes.
        assert!(decode_edit_v0(&full, 256).is_ok());
    }

    /// A hostile byte soup never panics, whatever the opcode or contents.
    #[test]
    fn arbitrary_bytes_never_panic() {
        for seed in 0u32..4096 {
            // A cheap xorshift-ish spread so the buffer contents vary in length
            // and value, including many that start with the AddShape opcode.
            let len = (seed % 40) as usize;
            let mut buf = Vec::with_capacity(len);
            let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
            for _ in 0..len {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                buf.push((x & 0xFF) as u8);
            }
            // Force some to start with a valid opcode to exercise the AddShape path.
            if seed % 3 == 0 && !buf.is_empty() {
                buf[0] = OP_ADD_SHAPE;
            }
            let _ = decode_edit_v0(&buf, 256);
        }
    }

    /// Appends a v0 length-prefixed name (`u16` length ++ UTF-8 bytes).
    fn enc_name(out: &mut Vec<u8>, name: &str) {
        let len = u16::try_from(name.len()).expect("test name fits in u16");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }

    /// Appends a v0 transform (`u8` orientation ++ `i32` tx,ty ++ `u32` num,den).
    fn enc_transform(out: &mut Vec<u8>, orient: u8, tx: i32, ty: i32, num: u32, den: u32) {
        out.push(orient);
        out.extend_from_slice(&tx.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&num.to_le_bytes());
        out.extend_from_slice(&den.to_le_bytes());
    }

    /// A full, valid record for each opcode (used by the truncation sweep).
    fn every_opcode_record() -> Vec<Vec<u8>> {
        let mut add_cell = vec![OP_ADD_CELL];
        enc_name(&mut add_cell, "SUB");

        let mut rm_cell = vec![OP_REMOVE_CELL];
        enc_name(&mut rm_cell, "SUB");

        let mut rm_shape = vec![OP_REMOVE_SHAPE];
        enc_name(&mut rm_shape, "TOP");
        rm_shape.extend_from_slice(&1u32.to_le_bytes());

        let mut inst = vec![OP_ADD_INSTANCE];
        enc_name(&mut inst, "TOP");
        enc_name(&mut inst, "CHILD");
        enc_transform(&mut inst, 1, 1, 2, 1, 1);

        let mut arr = vec![OP_ADD_ARRAY];
        enc_name(&mut arr, "TOP");
        enc_name(&mut arr, "BIT");
        enc_transform(&mut arr, 0, 0, 0, 1, 1);
        arr.extend_from_slice(&2u32.to_le_bytes());
        arr.extend_from_slice(&2u32.to_le_bytes());
        arr.extend_from_slice(&5i32.to_le_bytes());
        arr.extend_from_slice(&5i32.to_le_bytes());

        let mut label = vec![OP_ADD_LABEL];
        enc_name(&mut label, "TOP");
        enc_name(&mut label, "VDD");
        label.extend_from_slice(&0i32.to_le_bytes());
        label.extend_from_slice(&0i32.to_le_bytes());
        label.extend_from_slice(&0u16.to_le_bytes());
        label.extend_from_slice(&0u16.to_le_bytes());
        label.push(0);

        let mut rm_label = vec![OP_REMOVE_LABEL];
        enc_name(&mut rm_label, "TOP");
        rm_label.extend_from_slice(&0u32.to_le_bytes());

        vec![add_cell, rm_cell, rm_shape, inst, arr, label, rm_label]
    }

    /// An out-of-range transform orientation code is a clean error, not a panic.
    #[test]
    fn rejects_bad_orientation() {
        let mut b = vec![OP_ADD_INSTANCE];
        enc_name(&mut b, "TOP");
        enc_name(&mut b, "CHILD");
        enc_transform(&mut b, 8, 0, 0, 1, 1); // code 8 is outside 0..8
        assert_eq!(
            decode_edit_v0(&b, 256).unwrap_err(),
            EditDecodeError::BadOrientation(8)
        );
    }

    /// A zero magnification denominator is a clean error (the ratio is undefined).
    #[test]
    fn rejects_zero_magnification_denominator() {
        let mut b = vec![OP_ADD_INSTANCE];
        enc_name(&mut b, "TOP");
        enc_name(&mut b, "CHILD");
        enc_transform(&mut b, 0, 0, 0, 1, 0); // den = 0
        assert_eq!(
            decode_edit_v0(&b, 256).unwrap_err(),
            EditDecodeError::ZeroMagDenominator
        );
    }

    /// An unknown label anchor code is a clean error.
    #[test]
    fn rejects_bad_anchor() {
        let mut b = vec![OP_ADD_LABEL];
        enc_name(&mut b, "TOP");
        enc_name(&mut b, "VDD");
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(9); // anchor 9 is not in 0..5
        assert_eq!(
            decode_edit_v0(&b, 256).unwrap_err(),
            EditDecodeError::BadAnchor(9)
        );
    }

    /// Every strict prefix of a full record for EVERY opcode errors rather than
    /// panicking: the count-against-remaining discipline holds across the whole
    /// vocabulary, and the full record decodes.
    #[test]
    fn every_truncation_of_every_opcode_errors() {
        for full in every_opcode_record() {
            for n in 0..full.len() {
                assert!(
                    decode_edit_v0(&full[..n], 256).is_err(),
                    "opcode {:#04x}: prefix of length {n} must not decode",
                    full[0]
                );
            }
            assert!(
                decode_edit_v0(&full, 256).is_ok(),
                "opcode {:#04x}: the full record must decode",
                full[0]
            );
        }
    }
}
