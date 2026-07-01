//! Camera-to-clip projection.
//!
//! The renderer maps integer database-unit (DBU) world coordinates to wgpu clip
//! space with an orthographic projection derived from a [`Camera`] and the output
//! pixel size. World `+y` points up; the readback path flips rows so image row 0 is
//! the top of the screen (see [`crate::target`]).

use bytemuck::{Pod, Zeroable};
use reticle_model::Camera;

/// The uniform block uploaded to the shader: a column-major world-DBU -> clip-space
/// matrix. Matches `View` in `shapes.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct ViewUniform {
    /// Column-major `clip_from_world` transform.
    pub clip_from_world: [[f32; 4]; 4],
}

impl ViewUniform {
    /// Builds the projection for `camera` rendering into a `width` x `height` pixel
    /// target.
    ///
    /// The visible world rectangle is centered on [`Camera::center`] and sized so
    /// that one DBU spans [`Camera::pixels_per_dbu`] pixels. A degenerate camera
    /// (zero size or non-positive scale) falls back to an identity-like view so a
    /// frame still renders rather than producing NaNs.
    #[must_use]
    pub fn from_camera(camera: &Camera, width: u32, height: u32) -> Self {
        let w = width.max(1) as f32;
        let h = height.max(1) as f32;
        let ppd = if camera.pixels_per_dbu > 0.0 {
            camera.pixels_per_dbu
        } else {
            1.0
        };
        // Half-extents of the visible world rectangle, in DBU.
        let half_w = w / (2.0 * ppd);
        let half_h = h / (2.0 * ppd);
        let cx = camera.center.x as f32;
        let cy = camera.center.y as f32;

        // Right-handed, +y-up orthographic projection producing wgpu clip space
        // (Z in [0, 1], Y-up); `glam` is column-major, matching WGSL. This maps the
        // visible world rectangle onto the clip cube.
        let projection = glam::camera::rh::proj::directx::orthographic(
            cx - half_w,
            cx + half_w,
            cy - half_h,
            cy + half_h,
            0.0,
            1.0,
        );
        Self {
            clip_from_world: projection.to_cols_array_2d(),
        }
    }
}
