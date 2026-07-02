//! Offscreen render target and readback.
//!
//! [`OffscreenTarget`] wraps an `Rgba8Unorm` color texture sized in pixels, plus the
//! staging buffer used to copy it back to the CPU. Readback returns tightly packed
//! (row-unpadded) RGBA bytes with image row 0 at the top of the screen.
//!
//! The offscreen path renders multisampled: when the device supports 4x MSAA on
//! [`TARGET_FORMAT`], the target also allocates a `sample_count`-4 color texture and
//! the render pass resolves into the single-sample texture that backs readback. This
//! anti-aliases shape edges without changing the readback contract ([`view`] and
//! [`read_pixels`] always refer to the resolved single-sample image). Callers that
//! draw single-sample geometry (for example the retained-renderer tests) bind
//! [`view`] with no resolve target and ignore the MSAA companion.
//!
//! `copy_texture_to_buffer` requires each buffer row to be a multiple of
//! [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`] (256) bytes, so the staging buffer is
//! padded per row and the padding is stripped during readback.
//!
//! [`view`]: OffscreenTarget::view
//! [`read_pixels`]: OffscreenTarget::read_pixels

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

/// The preferred multisample count for the offscreen path. Used when the device
/// reports 4x MSAA support for [`TARGET_FORMAT`]; otherwise the target falls back to
/// single-sampled rendering.
pub const OFFSCREEN_SAMPLE_COUNT: u32 = 4;

/// An offscreen `Rgba8Unorm` render target with a CPU readback path.
///
/// When [`sample_count`](OffscreenTarget::sample_count) is greater than 1 the target
/// owns a multisampled color texture ([`msaa_view`](OffscreenTarget::msaa_view)) that
/// the render pass draws into and resolves down to the single-sample `texture` that
/// readback reads.
pub struct OffscreenTarget {
    texture: Texture,
    view: TextureView,
    /// Multisampled color texture, present only when `sample_count > 1`. The offscreen
    /// render pass draws into its view and resolves into `texture`.
    msaa_view: Option<TextureView>,
    readback: Buffer,
    width: u32,
    height: u32,
    sample_count: u32,
    padded_bytes_per_row: u32,
}

impl core::fmt::Debug for OffscreenTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OffscreenTarget")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("sample_count", &self.sample_count)
            .finish_non_exhaustive()
    }
}

impl OffscreenTarget {
    /// Creates a target of `width` x `height` pixels (each clamped to at least 1) on
    /// `ctx`'s device, multisampled at [`OFFSCREEN_SAMPLE_COUNT`] when the device
    /// supports it.
    ///
    /// The MSAA level is negotiated against the adapter's reported
    /// [`TextureFormatFeatureFlags`](wgpu::TextureFormatFeatureFlags) for
    /// [`TARGET_FORMAT`], so a device without 4x support silently renders
    /// single-sampled rather than failing.
    #[must_use]
    pub fn new(ctx: &WgpuContext, width: u32, height: u32) -> Self {
        let samples = if supports_4x_msaa(ctx) {
            OFFSCREEN_SAMPLE_COUNT
        } else {
            1
        };
        Self::with_sample_count(ctx, width, height, samples)
    }

    /// Creates a target with an explicit `sample_count` (clamped to at least 1).
    ///
    /// A `sample_count` of 1 is a plain single-sample target; a higher count adds the
    /// multisampled color texture and resolve path. The caller is responsible for
    /// pairing this with pipelines built at the same sample count.
    #[must_use]
    pub fn with_sample_count(
        ctx: &WgpuContext,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let sample_count = sample_count.max(1);
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

        // The multisampled color texture is a transient render attachment: it is never
        // copied from directly (readback reads the resolved `texture`), so it needs
        // only `RENDER_ATTACHMENT`.
        let msaa_view = (sample_count > 1).then(|| {
            let msaa = device.create_texture(&TextureDescriptor {
                label: Some("reticle-render offscreen msaa"),
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: TextureDimension::D2,
                format: TARGET_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            msaa.create_view(&TextureViewDescriptor::default())
        });

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
            msaa_view,
            readback,
            width,
            height,
            sample_count,
            padded_bytes_per_row,
        }
    }

    /// The single-sample color texture readback reads from (the resolve target when
    /// multisampled).
    #[must_use]
    pub fn texture(&self) -> &Texture {
        &self.texture
    }

    /// A view of the single-sample color texture, for use as a render attachment (or
    /// the resolve target when multisampled). This is always the image
    /// [`read_pixels`](OffscreenTarget::read_pixels) returns.
    #[must_use]
    pub fn view(&self) -> &TextureView {
        &self.view
    }

    /// The multisampled color view when this target is multisampled, else `None`.
    ///
    /// The offscreen render pass binds this as its color attachment and sets
    /// [`view`](OffscreenTarget::view) as the resolve target. A single-sample target
    /// returns `None`, and the caller draws straight into [`view`](OffscreenTarget::view).
    #[must_use]
    pub fn msaa_view(&self) -> Option<&TextureView> {
        self.msaa_view.as_ref()
    }

    /// The number of samples per pixel this target renders at (1 when not multisampled).
    #[must_use]
    pub fn sample_count(&self) -> u32 {
        self.sample_count
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

/// Whether `ctx`'s adapter supports 4x MSAA for [`TARGET_FORMAT`] as a render
/// attachment. Queried from the adapter's texture-format features so the offscreen
/// path only asks for a sample count the device can actually allocate.
pub(crate) fn supports_4x_msaa(ctx: &WgpuContext) -> bool {
    ctx.adapter()
        .get_texture_format_features(TARGET_FORMAT)
        .flags
        .sample_count_supported(OFFSCREEN_SAMPLE_COUNT)
}
