//! The GPU renderer for Reticle.
//!
//! This crate builds the `wgpu` renderer. Wave 2 lands a real, working *offscreen*
//! renderer: it acquires a headless GPU device ([`WgpuContext`]), draws the shapes of
//! a cell into an [`OffscreenTarget`] (axis-aligned rectangles as instanced quads,
//! polygons and paths tessellated with `lyon`), and reads the pixels back to the CPU.
//! Colors come from the technology layer table with a fallback palette, and a
//! [`Camera`]-derived orthographic projection maps database-unit world coordinates to
//! clip space.
//!
//! A compute-shader cell culler ([`CellCuller`]) is also included: it flags which
//! cell bounding boxes overlap the viewport on the GPU, the first stage of a
//! GPU-driven draw list. Later waves add surface/window presentation, compaction of
//! culled cells into indirect draws, a tile/LOD pyramid, anti-aliased edges, a
//! `glyphon` glyph atlas, and overlays.
//!
//! # Entry points
//!
//! - [`WgpuContext::new_blocking`] (native) or [`WgpuContext::new`] (async) to get a
//!   device.
//! - [`WgpuRenderer::render_document_offscreen`] to render a top cell and get back
//!   RGBA bytes.
//! - [`WgpuRenderer`] also implements the `reticle-model` [`Renderer`] trait (Wave 0
//!   contract); that method is a lightweight frame counter until surface presentation
//!   arrives in a later wave.

mod context;
mod cull;
mod geometry;
mod indirect;
mod pages;
mod palette;
mod pipeline3d;
mod pipelines;
mod retained;
mod surface;
mod target;
mod view;

pub use context::WgpuContext;
pub use cull::{CellCompactor, CellCuller, CompactionOutput, CullAabb, QUAD_INDEX_COUNT};
pub use geometry::{MeshVertex, RectInstance, SceneGeometry};
pub use indirect::{IndirectRects, MultiDraw, upload_instances};
pub use pages::{Allocation, BufferPages, DEFAULT_PAGE_SIZE, PageAllocator};
pub use palette::{Palette, Rgba};
pub use pipeline3d::{
    BlitPipeline, DEPTH_FORMAT, LIGHT_DIR, LayerSpan, Mesh3d, OrbitCamera, RenderTarget3d,
    StackRenderer, StackUniform, StackView, Vertex3d, layer_spans, render_stack_offscreen,
};
pub use pipelines::Pipelines;
pub use retained::{
    CellChunk, ExpandedScene, InstanceEntry, InstanceTransform, RectInstanceT, RetainedScene,
};
pub use surface::RetainedRenderer;
pub use target::{OFFSCREEN_SAMPLE_COUNT, OffscreenTarget, TARGET_FORMAT};
pub use view::ViewUniform;

use reticle_model::{Camera, Document, Renderer};

/// The default clear color for offscreen frames: opaque black.
pub const DEFAULT_CLEAR: Rgba = Rgba {
    components: [0.0, 0.0, 0.0, 1.0],
};

/// The `wgpu`-based renderer.
///
/// Holds a frame counter for the Wave 0 [`Renderer`] contract; the offscreen path
/// ([`WgpuRenderer::render_document_offscreen`]) is stateless with respect to the
/// renderer and takes the GPU context explicitly.
#[derive(Debug, Default)]
pub struct WgpuRenderer {
    frames_rendered: u64,
    clear: Option<Rgba>,
}

impl WgpuRenderer {
    /// Creates a renderer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of frames rendered so far (via the [`Renderer`] trait method).
    #[must_use]
    pub fn frames_rendered(&self) -> u64 {
        self.frames_rendered
    }

    /// Overrides the clear color used by [`WgpuRenderer::render_document_offscreen`].
    /// Defaults to [`DEFAULT_CLEAR`].
    pub fn set_clear_color(&mut self, clear: Rgba) {
        self.clear = Some(clear);
    }

    /// The clear color offscreen frames will use.
    #[must_use]
    pub fn clear_color(&self) -> Rgba {
        self.clear.unwrap_or(DEFAULT_CLEAR)
    }

    /// Renders `top_cell` of `doc` as seen through `camera` into an offscreen RGBA8
    /// target of `size` (`width`, `height`) pixels, returning tightly packed RGBA
    /// bytes (`width * height * 4`), image row 0 at the top.
    ///
    /// The cell is flattened, its shapes are converted to GPU geometry (rectangles
    /// instanced, polygons/paths tessellated), colored through the technology layer
    /// table, and drawn over the clear color. Invisible layers are skipped. Returns
    /// an all-clear image if `top_cell` is missing or empty.
    ///
    /// This builds fresh pipelines and an offscreen target per call, so it is meant
    /// for one-shot rendering (tests, thumbnails, export); an interactive loop should
    /// keep [`Pipelines`] and [`OffscreenTarget`] alive across frames.
    #[must_use]
    pub fn render_document_offscreen(
        &mut self,
        ctx: &WgpuContext,
        doc: &Document,
        top_cell: &str,
        camera: &Camera,
        size: (u32, u32),
    ) -> Vec<u8> {
        let (width, height) = size;
        let pipelines = Pipelines::new(ctx);
        let target = OffscreenTarget::new(ctx, width, height);

        let palette = Palette::from_technology(doc.technology());
        let shapes = doc.flatten(top_cell);
        let geometry = SceneGeometry::build(&shapes, &palette);
        let view = ViewUniform::from_camera(camera, target.width(), target.height());

        pipelines.render(ctx, &target, &geometry, &view, self.clear_color());
        self.frames_rendered += 1;
        target.read_pixels(ctx)
    }
}

impl Renderer for WgpuRenderer {
    fn render(&mut self, _doc: &Document, _camera: &Camera) {
        // Surface presentation is a later wave; the offscreen path is
        // `render_document_offscreen`. Tick so the contract stays observable.
        self.frames_rendered += 1;
    }
}
