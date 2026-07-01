//! Offscreen render target and readback.
//!
//! [`OffscreenTarget`] wraps an `Rgba8Unorm` color texture sized in pixels, plus the
//! staging buffer used to copy it back to the CPU. Readback returns tightly packed
//! (row-unpadded) RGBA bytes with image row 0 at the top of the screen.
//!
//! `copy_texture_to_buffer` requires each buffer row to be a multiple of
//! [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`] (256) bytes, so the staging buffer is
//! padded per row and the padding is stripped during readback.

use crate::context::WgpuContext;
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, Extent3d, MapMode, Origin3d, PollType,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

/// The color format of the offscreen target. Non-sRGB so shader colors round-trip
/// to exact bytes (see [`Palette`](crate::Palette)).
pub const TARGET_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// Bytes per pixel for [`TARGET_FORMAT`].
const BYTES_PER_PIXEL: u32 = 4;

/// An offscreen `Rgba8Unorm` render target with a CPU readback path.
pub struct OffscreenTarget {
    texture: Texture,
    view: TextureView,
    readback: Buffer,
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
}

impl core::fmt::Debug for OffscreenTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OffscreenTarget")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl OffscreenTarget {
    /// Creates a target of `width` x `height` pixels (each clamped to at least 1)
    /// on `ctx`'s device.
    #[must_use]
    pub fn new(ctx: &WgpuContext, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let device = ctx.device();

        let texture = device.create_texture(&TextureDescriptor {
            label: Some("reticle-render offscreen"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());

        let padded_bytes_per_row = padded_bytes_per_row(width);
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("reticle-render readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            texture,
            view,
            readback,
            width,
            height,
            padded_bytes_per_row,
        }
    }

    /// The color texture.
    #[must_use]
    pub fn texture(&self) -> &Texture {
        &self.texture
    }

    /// A default view of the color texture, for use as a render attachment.
    #[must_use]
    pub fn view(&self) -> &TextureView {
        &self.view
    }

    /// Target width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Target height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Records a copy of the color texture into the staging buffer on `encoder`.
    ///
    /// Submit the encoder before calling [`OffscreenTarget::read_pixels`].
    pub fn copy_to_buffer(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Maps the staging buffer and returns tightly packed RGBA bytes
    /// (`width * height * 4`), row 0 at the top.
    ///
    /// Blocks on `ctx`'s device to drive the map to completion, so it must run after
    /// the copy submitted via [`OffscreenTarget::copy_to_buffer`] has been queued.
    #[must_use]
    pub fn read_pixels(&self, ctx: &WgpuContext) -> Vec<u8> {
        let slice = self.readback.slice(..);
        slice.map_async(MapMode::Read, |_| {});
        // Drive the queue until the map callback fires.
        let _ = ctx.device().poll(PollType::wait_indefinitely());

        let padded = self.padded_bytes_per_row as usize;
        let unpadded = (self.width * BYTES_PER_PIXEL) as usize;
        let mut out = Vec::with_capacity(unpadded * self.height as usize);
        {
            let data = slice.get_mapped_range();
            for row in 0..self.height as usize {
                let start = row * padded;
                out.extend_from_slice(&data[start..start + unpadded]);
            }
        }
        self.readback.unmap();
        out
    }
}

/// Rounds a row width in bytes up to [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`].
fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * BYTES_PER_PIXEL;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    unpadded.div_ceil(align) * align
}
