//! The interactive Reticle application.
//!
//! Wave 4 builds the full editing suite on `egui`: the tool state machine, command
//! palette, rebindable keybinds, a config file, multi-viewport split, a layer
//! manager with search, selection filters and a query bar, rulers, grid, snap,
//! guides, a measurement suite, session save/restore, autosave and crash recovery,
//! and an undo-history panel. It runs both native and in the browser (WASM).
//!
//! The Wave 0 contract is [`App`], which already wires the render + model + sync
//! subsystems together so the integration surface compiles.

use reticle_render::WgpuRenderer;
use reticle_sync::SyncDocument;

/// The top-level application state: the collaborative document and the renderer.
#[derive(Debug, Default)]
pub struct App {
    renderer: WgpuRenderer,
    document: SyncDocument,
}

impl App {
    /// Creates a new application with an empty document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            renderer: WgpuRenderer::new(),
            document: SyncDocument::new("local"),
        }
    }

    /// The renderer.
    #[must_use]
    pub fn renderer(&self) -> &WgpuRenderer {
        &self.renderer
    }

    /// The collaborative document.
    #[must_use]
    pub fn document(&self) -> &SyncDocument {
        &self.document
    }
}
