//! On-surface retained rendering for the windowed (eframe) path.
//!
//! [`RetainedRenderer`] is the GPU-resident half of the interactive renderer. It is
//! built once on eframe's shared device for the surface's color format, then stored
//! in the egui-wgpu callback resources and driven each frame:
//!
//! * [`RetainedRenderer::sync`] uploads the expanded scene geometry, but only when
//!   the caller's revision token changes. Geometry lands in [`BufferPages`] via
//!   `queue.write_buffer`, reusing prior allocations; nothing is rebuilt on a plain
//!   camera move.
//! * [`RetainedRenderer::set_camera`] rewrites just the view uniform (one
//!   `write_buffer`), so panning and zooming never touch geometry buffers.
//! * [`RetainedRenderer::paint`] records the draw into egui's render pass, so the
//!   scene composites under egui's own overlays (selection, DRC, rulers).
//!
//! Rects carry their placement transform per instance and are drawn with the
//! retained pipeline; polygons and paths are a transform-baked triangle mesh. The
//! geometry buffers are sized to hold the whole expanded scene in a single page each
//! (the page grows only when the scene grows), so every draw is one contiguous
//! buffer slice.

use crate::pages::{Allocation, BufferPages, DEFAULT_PAGE_SIZE};
use crate::pipelines::Pipelines;
use crate::retained::{ExpandedScene, RetainedScene};
use crate::view::ViewUniform;
use bytemuck::{bytes_of, cast_slice};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, Buffer, BufferDescriptor, BufferUsages, Device,
    IndexFormat, Queue, RenderPass, TextureFormat,
};

/// A GPU-resident retained renderer: pipelines, a persistent camera uniform, and
/// paged geometry buffers, sized for one surface color format.
pub struct RetainedRenderer {
    pipelines: Pipelines,
    /// The view (camera) uniform buffer, rewritten in place every frame.
    view_buffer: Buffer,
    /// Bind group for `view_buffer` at group 0.
    view_bind_group: BindGroup,
    /// Paged storage for retained rect instances.
    rect_pages: BufferPages,
    /// Paged storage for transform-baked mesh vertices.
    vertex_pages: BufferPages,
    /// Paged storage for mesh indices.
    index_pages: BufferPages,
    /// Current rect-instance allocation and count, if any geometry is uploaded.
    rect_alloc: Option<Allocation>,
    rect_count: u32,
    /// Current mesh vertex/index allocations and the index count.
    vertex_alloc: Option<Allocation>,
    index_alloc: Option<Allocation>,
    index_count: u32,
    /// The scene revision the uploaded geometry reflects. `sync` is a no-op while the
    /// revision is unchanged, so geometry uploads happen only on real edits.
    uploaded_revision: Option<u64>,
}

impl core::fmt::Debug for RetainedRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetainedRenderer")
            .field("format", &self.pipelines.format())
            .field("rects", &self.rect_count)
            .field("indices", &self.index_count)
            .field("uploaded_revision", &self.uploaded_revision)
            .finish_non_exhaustive()
    }
}

impl RetainedRenderer {
    /// Builds the renderer on `device` for the surface color `format`.
    ///
    /// This is the [`Pipelines::for_format`] path: the offscreen renderer keeps its
    /// own device and [`crate::TARGET_FORMAT`]; this shares eframe's device and its
    /// surface format so the callback can draw into egui's pass.
    #[must_use]
    pub fn new(device: &Device, format: TextureFormat) -> Self {
        let pipelines = Pipelines::for_format(device, format);

        let view_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render surface view uniform"),
            size: std::mem::size_of::<ViewUniform>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("reticle-render surface view bind group"),
            layout: pipelines.uniform_layout(),
            entries: &[BindGroupEntry {
                binding: 0,
                resource: view_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipelines,
            view_buffer,
            view_bind_group,
            rect_pages: BufferPages::new(DEFAULT_PAGE_SIZE, BufferUsages::VERTEX, "retained rects"),
            vertex_pages: BufferPages::new(
                DEFAULT_PAGE_SIZE,
                BufferUsages::VERTEX,
                "retained mesh verts",
            ),
            index_pages: BufferPages::new(
                DEFAULT_PAGE_SIZE,
                BufferUsages::INDEX,
                "retained indices",
            ),
            rect_alloc: None,
            rect_count: 0,
            vertex_alloc: None,
            index_alloc: None,
            index_count: 0,
            uploaded_revision: None,
        }
    }

    /// The color format this renderer targets.
    #[must_use]
    pub fn format(&self) -> TextureFormat {
        self.pipelines.format()
    }

    /// Uploads the scene's geometry if `revision` differs from the last upload.
    ///
    /// On a revision change the scene is expanded once (rects with per-instance
    /// transforms, a transform-baked mesh) and written into the paged buffers,
    /// reusing prior allocations. Unchanged revisions return immediately, so this is
    /// safe (and cheap) to call every frame.
    pub fn sync(&mut self, device: &Device, queue: &Queue, scene: &RetainedScene, revision: u64) {
        if self.uploaded_revision == Some(revision) {
            return;
        }
        let expanded = scene.expand();
        self.upload(device, queue, &expanded);
        self.uploaded_revision = Some(revision);
    }

    /// Uploads an already-expanded scene if `revision` differs from the last upload.
    ///
    /// The windowed path expands the scene on the CPU side (in the app) and hands the
    /// result plus a revision to the callback; this uploads it only when that revision
    /// changed, so a plain camera move re-uses the GPU buffers untouched.
    pub fn sync_expanded(
        &mut self,
        device: &Device,
        queue: &Queue,
        expanded: &ExpandedScene,
        revision: u64,
    ) {
        if self.uploaded_revision == Some(revision) {
            return;
        }
        self.upload(device, queue, expanded);
        self.uploaded_revision = Some(revision);
    }

    /// Forces a re-upload of `expanded` on the next paint regardless of revision.
    /// Used by tests and callers that hold an already-expanded scene.
    pub fn upload_expanded(&mut self, device: &Device, queue: &Queue, expanded: &ExpandedScene) {
        self.upload(device, queue, expanded);
        self.uploaded_revision = None;
    }

    /// Uploads expanded geometry into the paged buffers, growing a page only when the
    /// data no longer fits.
    fn upload(&mut self, device: &Device, queue: &Queue, expanded: &ExpandedScene) {
        // Rects.
        if expanded.rects.is_empty() {
            self.rect_count = 0;
        } else {
            let bytes: &[u8] = cast_slice(&expanded.rects);
            self.rect_alloc =
                Self::reupload(&mut self.rect_pages, device, queue, self.rect_alloc, bytes);
            self.rect_count = u32::try_from(expanded.rects.len()).unwrap_or(u32::MAX);
        }

        // Mesh vertices + indices.
        if expanded.mesh_indices.is_empty() {
            self.index_count = 0;
        } else {
            let vbytes: &[u8] = cast_slice(&expanded.mesh_vertices);
            self.vertex_alloc = Self::reupload(
                &mut self.vertex_pages,
                device,
                queue,
                self.vertex_alloc,
                vbytes,
            );
            let ibytes: &[u8] = cast_slice(&expanded.mesh_indices);
            self.index_alloc = Self::reupload(
                &mut self.index_pages,
                device,
                queue,
                self.index_alloc,
                ibytes,
            );
            self.index_count = u32::try_from(expanded.mesh_indices.len()).unwrap_or(u32::MAX);
        }
    }

    /// Reuploads `bytes` into `pages`, reusing `prev` when it still fits and freeing
    /// then reallocating (growing the page size if needed) when it does not. Returns
    /// the allocation the data now lives in.
    fn reupload(
        pages: &mut BufferPages,
        device: &Device,
        queue: &Queue,
        prev: Option<Allocation>,
        bytes: &[u8],
    ) -> Option<Allocation> {
        let need = bytes.len() as u64;
        // Reuse the prior allocation in place when it is large enough.
        if let Some(alloc) = prev
            && alloc.len() >= need
        {
            pages.write(queue, &alloc, bytes);
            return Some(alloc);
        }
        // Otherwise free the old range and (re)allocate. If a single allocation would
        // exceed the current page size, rebuild the bank with a page big enough to
        // hold the whole buffer in one page (this only happens as the scene grows,
        // never on a camera move).
        if let Some(alloc) = prev {
            pages.free(alloc);
        }
        if need > pages.page_size() {
            let bigger = need.next_power_of_two().max(DEFAULT_PAGE_SIZE);
            *pages = pages.with_page_size(bigger);
        }
        pages.upload(device, queue, bytes)
    }

    /// Rewrites the view (camera) uniform in place. One `write_buffer`, no geometry
    /// touched, so panning and zooming stay uniform-only.
    pub fn set_camera(&self, queue: &Queue, view: &ViewUniform) {
        queue.write_buffer(&self.view_buffer, 0, bytes_of(view));
    }

    /// Records the retained scene into `pass` (egui's render pass), so the geometry
    /// draws beneath egui's overlays. Assumes [`RetainedRenderer::set_camera`] and
    /// [`RetainedRenderer::sync`] already ran this frame.
    ///
    /// The pass lifetime is independent of `&self`: wgpu render-pass encoding holds
    /// its own references to the bound buffers and pipelines, so the renderer does
    /// not need to outlive the pass. This is what lets egui-wgpu hand us a
    /// `RenderPass<'static>` while our resources are borrowed only for the call.
    pub fn paint(&self, pass: &mut RenderPass<'_>) {
        pass.set_bind_group(0, &self.view_bind_group, &[]);

        if self.rect_count > 0
            && let Some(alloc) = self.rect_alloc
            && let Some(buffer) = self.rect_pages.page_buffer(alloc.page())
        {
            let end = alloc.offset() + alloc.len();
            pass.set_pipeline(self.pipelines.retained_rect_pipeline());
            pass.set_vertex_buffer(0, buffer.slice(alloc.offset()..end));
            // Four vertices per instanced unit quad (triangle strip).
            pass.draw(0..4, 0..self.rect_count);
        }

        if self.index_count > 0
            && let (Some(va), Some(ia)) = (self.vertex_alloc, self.index_alloc)
            && let (Some(vbuf), Some(ibuf)) = (
                self.vertex_pages.page_buffer(va.page()),
                self.index_pages.page_buffer(ia.page()),
            )
        {
            let vend = va.offset() + va.len();
            let iend = ia.offset() + ia.len();
            pass.set_pipeline(self.pipelines.mesh_pipeline());
            pass.set_vertex_buffer(0, vbuf.slice(va.offset()..vend));
            pass.set_index_buffer(ibuf.slice(ia.offset()..iend), IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }

    /// The number of retained rect instances currently uploaded.
    #[must_use]
    pub fn rect_count(&self) -> u32 {
        self.rect_count
    }

    /// The number of mesh indices currently uploaded.
    #[must_use]
    pub fn index_count(&self) -> u32 {
        self.index_count
    }
}

#[cfg(test)]
mod tests {
    // A guard that MeshVertex stays the size the vertex layout assumes (2 f32
    // position + 4 f32 color = 24 bytes).
    #[test]
    fn mesh_vertex_layout_is_stable() {
        assert_eq!(std::mem::size_of::<crate::MeshVertex>(), 24);
    }

    #[test]
    fn rect_instance_t_is_tightly_packed() {
        // 2+2 f32 corners + 4 f32 color + u32 + f32 + 2 i32 = 48 bytes, no padding.
        assert_eq!(std::mem::size_of::<crate::RectInstanceT>(), 48);
    }
}
