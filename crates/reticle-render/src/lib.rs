//! The GPU renderer for Reticle.
//!
//! Wave 2 builds the `wgpu` renderer: instanced polygon/path pipelines, a compute
//! shader that builds the visible draw list (GPU-driven culling), a tile and LOD
//! pyramid, anti-aliased edges, a `glyphon` glyph atlas, layer styling and themes,
//! a minimap, DRC/net overlays, and an optional 3D layer-stack cross-section. It
//! renders to a surface or an offscreen texture.
//!
//! The Wave 0 contract is [`WgpuRenderer`], a no-op implementation of the
//! `reticle-model` [`Renderer`] trait so the app and CLI can compile against it.

use reticle_model::{Camera, Document, Renderer};

/// The wgpu-based renderer (Wave 2). The Wave 0 placeholder counts frames so the
/// [`Renderer`] contract is observable without a GPU.
#[derive(Debug, Default)]
pub struct WgpuRenderer {
    frames_rendered: u64,
}

impl WgpuRenderer {
    /// Creates a renderer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of frames rendered so far.
    #[must_use]
    pub fn frames_rendered(&self) -> u64 {
        self.frames_rendered
    }
}

impl Renderer for WgpuRenderer {
    fn render(&mut self, _doc: &Document, _camera: &Camera) {
        // Wave 2: cull on the GPU, build instance buffers, draw. For now just tick.
        self.frames_rendered += 1;
    }
}
