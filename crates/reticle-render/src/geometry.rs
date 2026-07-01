//! CPU-side geometry preparation.
//!
//! Turns the flattened [`DrawShape`]s of a cell into GPU-ready buffers: axis-aligned
//! rectangles become per-instance quads ([`RectInstance`]), while polygons and paths
//! are tessellated with `lyon` into an indexed triangle mesh ([`MeshVertex`] +
//! `u32` indices). Colors are resolved through the [`Palette`] and invisible layers
//! are skipped.

use crate::palette::Palette;
use bytemuck::{Pod, Zeroable};
use lyon::math::{Point as LyonPoint, point};
use lyon::path::polygon::Polygon as LyonPolygon;
use lyon::path::{LineCap, Path as LyonPath};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use reticle_geometry::{Endcap, Point, Rect};
use reticle_model::{DrawShape, ShapeKind};

/// Per-instance data for an axis-aligned rectangle: world-space (DBU) min/max
/// corners and a linear RGBA color. Matches `RectInstance` in `shapes.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct RectInstance {
    /// Minimum corner `(x, y)` in DBU.
    pub min_xy: [f32; 2],
    /// Maximum corner `(x, y)` in DBU.
    pub max_xy: [f32; 2],
    /// Linear RGBA fill color.
    pub color: [f32; 4],
}

/// Per-vertex data for a tessellated mesh: world-space (DBU) position and a linear
/// RGBA color. Matches `MeshVertex` in `shapes.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    /// World-space position `(x, y)` in DBU.
    pub position: [f32; 2],
    /// Linear RGBA color.
    pub color: [f32; 4],
}

/// The GPU-ready geometry for a frame: instanced rectangles plus an indexed
/// triangle mesh for everything that needed tessellation.
#[derive(Clone, Default, Debug)]
pub struct SceneGeometry {
    /// One entry per axis-aligned rectangle.
    pub rects: Vec<RectInstance>,
    /// Tessellated polygon/path vertices.
    pub mesh_vertices: Vec<MeshVertex>,
    /// Indices into `mesh_vertices` (triangle list).
    pub mesh_indices: Vec<u32>,
}

impl SceneGeometry {
    /// Returns `true` if there is nothing to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty() && self.mesh_indices.is_empty()
    }

    /// Builds scene geometry from flattened shapes, resolving colors and skipping
    /// invisible layers. Degenerate geometry that `lyon` cannot tessellate is
    /// silently dropped rather than aborting the frame.
    #[must_use]
    pub fn build(shapes: &[DrawShape], palette: &Palette) -> Self {
        let mut out = Self::default();
        let mut fill = FillTessellator::new();
        let mut stroke = StrokeTessellator::new();

        for shape in shapes {
            if !palette.is_visible(shape.layer) {
                continue;
            }
            let color = palette.color(shape.layer).components;
            match &shape.kind {
                ShapeKind::Rect(rect) => out.push_rect(*rect, color),
                ShapeKind::Polygon(poly) => {
                    out.push_polygon(poly.vertices(), color, &mut fill);
                }
                ShapeKind::Path(path) => {
                    out.push_path(
                        path.points(),
                        path.width(),
                        path.endcap(),
                        color,
                        &mut stroke,
                    );
                }
            }
        }
        out
    }

    fn push_rect(&mut self, rect: Rect, color: [f32; 4]) {
        if rect.is_empty() {
            return;
        }
        self.rects.push(RectInstance {
            min_xy: [rect.min.x as f32, rect.min.y as f32],
            max_xy: [rect.max.x as f32, rect.max.y as f32],
            color,
        });
    }

    fn push_polygon(&mut self, vertices: &[Point], color: [f32; 4], fill: &mut FillTessellator) {
        if vertices.len() < 3 {
            return;
        }
        let points: Vec<LyonPoint> = vertices
            .iter()
            .map(|p| point(p.x as f32, p.y as f32))
            .collect();
        let polygon = LyonPolygon {
            points: &points,
            closed: true,
        };
        let mut buffers: VertexBuffers<MeshVertex, u32> = VertexBuffers::new();
        let mut builder = BuffersBuilder::new(&mut buffers, |v: FillVertex| {
            let pos = v.position();
            MeshVertex {
                position: [pos.x, pos.y],
                color,
            }
        });
        if fill
            .tessellate_polygon(polygon, &FillOptions::default(), &mut builder)
            .is_ok()
        {
            self.append_mesh(&buffers);
        }
    }

    fn push_path(
        &mut self,
        points: &[Point],
        width: i32,
        endcap: Endcap,
        color: [f32; 4],
        stroke: &mut StrokeTessellator,
    ) {
        if points.len() < 2 || width <= 0 {
            return;
        }
        let mut builder = LyonPath::builder();
        builder.begin(point(points[0].x as f32, points[0].y as f32));
        for p in &points[1..] {
            builder.line_to(point(p.x as f32, p.y as f32));
        }
        builder.end(false);
        let path = builder.build();

        let cap = match endcap {
            Endcap::Round => LineCap::Round,
            Endcap::Square | Endcap::Custom(_) => LineCap::Square,
            Endcap::Flat => LineCap::Butt,
        };
        let options = StrokeOptions::default()
            .with_line_width(width as f32)
            .with_line_cap(cap);

        let mut buffers: VertexBuffers<MeshVertex, u32> = VertexBuffers::new();
        let mut out = BuffersBuilder::new(&mut buffers, |v: StrokeVertex| {
            let pos = v.position();
            MeshVertex {
                position: [pos.x, pos.y],
                color,
            }
        });
        if stroke.tessellate_path(&path, &options, &mut out).is_ok() {
            self.append_mesh(&buffers);
        }
    }

    /// Appends a tessellated buffer set, rebasing its indices onto the running
    /// vertex list.
    fn append_mesh(&mut self, buffers: &VertexBuffers<MeshVertex, u32>) {
        let base = u32::try_from(self.mesh_vertices.len()).unwrap_or(u32::MAX);
        self.mesh_vertices.extend_from_slice(&buffers.vertices);
        self.mesh_indices
            .extend(buffers.indices.iter().map(|i| i + base));
    }
}
