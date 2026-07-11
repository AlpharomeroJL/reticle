//! The image underlay: a positioned, scaled, opacity-controlled raster
//! backdrop rendered under the layout, for tracing/reference against a die
//! photo or datasheet figure (`underlay.load` / `underlay.align` /
//! `underlay.opacity`; ADR 0118).
//!
//! This module owns the pure, testable state (the decoded pixels, the
//! position/scale/opacity transform) and the two decode routes:
//!
//! * Native: [`UnderlayState::load`] decodes through
//!   [`reticle_render::decode_underlay_image`] (the `image` crate, PNG and
//!   JPEG).
//! * wasm32: `decode_via_browser` (below; not an intra-doc link, since the
//!   function only exists in a wasm32 build and this doc also builds
//!   natively) decodes through the browser's own image codec
//!   (`createImageBitmap` plus a detached-canvas readback) instead of
//!   shipping a Rust decoder in the wasm bundle. A byte measurement (`just
//!   bundle-gate`) found that even a PNG-only `image` dependency added about
//!   60 KiB gz (its unconditional `moxcms` color-management dependency ships
//!   regardless of which format feature is enabled), landing 43.4 KiB over
//!   the +450 KiB budget before JPEG was even considered; see
//!   `docs/decisions/0118-underlay-decode-avoids-image-crate-in-wasm.md` for
//!   the full measurement. The browser route is async (`createImageBitmap`
//!   returns a promise), so it is driven
//!   through the same pending-pick mailbox the file picker itself already
//!   needs (mirrors `crate::webopen::WebOpenInbox`); native reads and
//!   decodes synchronously instead.
//!
//! Texture upload, painting, and the file-dialog wiring are `egui`/GPU/DOM
//! concerns and live in `crate::app`.

use reticle_geometry::{Point, Rect};

pub use reticle_render::DecodedImage;
#[cfg(not(target_arch = "wasm32"))]
pub use reticle_render::UnderlayImageError;

/// The smallest scale (world DBU per source pixel) [`UnderlayTransform::set_scale`]
/// accepts. Guards against a zero, negative, or non-finite scale collapsing
/// or flipping the image.
pub const MIN_SCALE: f32 = 1.0e-4;

/// Position, scale, and opacity applied to the underlay image.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UnderlayTransform {
    /// World-space (DBU) x of the image's bottom-left corner.
    pub x: f32,
    /// World-space (DBU) y of the image's bottom-left corner.
    pub y: f32,
    /// World DBU per source-image pixel; always positive ([`MIN_SCALE`] floor).
    pub scale: f32,
    /// Blend opacity, `0.0..=1.0`.
    pub opacity: f32,
}

impl Default for UnderlayTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            opacity: 0.5,
        }
    }
}

impl UnderlayTransform {
    /// Moves the image's bottom-left corner to `(x, y)` in world DBU.
    pub fn set_position(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
    }

    /// Sets the world-DBU-per-source-pixel scale, floored at [`MIN_SCALE`]
    /// (and floored there too for non-finite input) so it can never reach
    /// zero, negative, or NaN.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = if scale.is_finite() {
            scale.max(MIN_SCALE)
        } else {
            MIN_SCALE
        };
    }

    /// Sets the blend opacity, clamped to `0.0..=1.0` (non-finite input
    /// floors to `0.0`, fully transparent, rather than propagating a NaN).
    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    /// The world-space (DBU) rectangle a `image_width`x`image_height` source
    /// image occupies at this transform: bottom-left corner at `(x, y)`,
    /// extending `image_width * scale` right and `image_height * scale` up.
    /// Rounds to the nearest DBU (the shared integer world grid; sub-DBU
    /// precision has no meaning for a reference backdrop).
    #[must_use]
    pub fn world_rect(&self, image_width: u32, image_height: u32) -> Rect {
        let w = image_width as f32 * self.scale;
        let h = image_height as f32 * self.scale;
        #[allow(clippy::cast_possible_truncation)]
        let min = Point::new(self.x.round() as i32, self.y.round() as i32);
        #[allow(clippy::cast_possible_truncation)]
        let max = Point::new((self.x + w).round() as i32, (self.y + h).round() as i32);
        Rect::new(min, max)
    }
}

/// A file the async browser picker read and decoded, or the failure message
/// if it could not be read, decoded, or was not a recognized image (mirrors
/// `crate::webopen::WebOpenEvent`, scoped to just this one picker). Carries
/// the already-decoded [`DecodedImage`] (not raw bytes): the browser decode
/// step (`createImageBitmap`) is itself async, so it runs inside the same
/// picker task that reads the file, before posting here.
pub type PickResult = Result<(String, DecodedImage), String>;

/// The wasm file-picker's pending-result mailbox.
///
/// `rfd::AsyncFileDialog`'s future is `'static` and cannot borrow `&mut App`,
/// so it posts its result here; `crate::app` drains it the next frame on the
/// main thread (exactly `crate::webopen::WebOpenInbox`'s pattern). Always
/// empty on native, where the picker reads and decodes the file inline
/// instead ([`UnderlayState::load`]); the type is uniform across targets so
/// call sites need no `cfg`.
#[derive(Clone, Debug, Default)]
pub struct PickMailbox {
    #[cfg(target_arch = "wasm32")]
    inner: std::rc::Rc<std::cell::RefCell<Option<PickResult>>>,
}

impl PickMailbox {
    /// Posts a result for the next frame to drain (wasm only; a no-op
    /// elsewhere so the type is uniform across targets).
    // On native this drops `result` immediately (a no-op body); on wasm32 it
    // is moved into the `RefCell`. Both `unused_self` and
    // `needless_pass_by_value` are right about the native body in isolation,
    // but the owned-by-value signature is what the wasm32 body actually
    // needs, so both are allowed rather than restructuring the signature
    // per target.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        allow(clippy::unused_self, clippy::needless_pass_by_value)
    )]
    pub fn post(&self, result: PickResult) {
        #[cfg(target_arch = "wasm32")]
        {
            *self.inner.borrow_mut() = Some(result);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = result;
        }
    }

    /// Takes the pending result, if the async picker has posted one since
    /// the last drain (wasm only; always `None` elsewhere).
    #[cfg_attr(not(target_arch = "wasm32"), allow(clippy::unused_self))]
    pub fn take(&self) -> Option<PickResult> {
        #[cfg(target_arch = "wasm32")]
        {
            self.inner.borrow_mut().take()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            None
        }
    }
}

/// The full underlay state: the decoded image (if any), its transform,
/// whether it paints, and the last load failure.
#[derive(Clone, Debug, Default)]
pub struct UnderlayState {
    image: Option<DecodedImage>,
    /// Position, scale, and opacity; mutated directly by the Inspector's
    /// sliders for continuous adjustment (the `underlay.align` /
    /// `underlay.opacity` commands apply discrete presets on top of this).
    pub transform: UnderlayTransform,
    /// Whether the underlay paints at all. Independent of whether an image
    /// is loaded, so hiding it keeps the image and transform and needs no
    /// reload to show it again.
    pub visible: bool,
    /// Bumped on every successful load, so `crate::app`'s texture cache
    /// knows to re-upload rather than doing so every frame.
    revision: u64,
    /// The last load failure's message, shown by the Inspector; cleared on
    /// the next successful load.
    pub last_error: Option<String>,
    /// The async browser picker's pending-result mailbox.
    pending: PickMailbox,
}

impl UnderlayState {
    /// Decodes `bytes` (untrusted input: see
    /// [`reticle_render::decode_underlay_image`]'s cap-before-allocate
    /// discipline) through the native `image` crate and, on success, adopts
    /// the result as the current image and makes it visible. On failure the
    /// previous image (if any) is left untouched and
    /// [`UnderlayState::last_error`] is set to the failure's message.
    ///
    /// Native only: the wasm32 browser picker decodes through
    /// `decode_via_browser` instead (async, not an intra-doc link here: it
    /// only exists in a wasm32 build) and adopts the result with
    /// [`UnderlayState::adopt_decoded`].
    ///
    /// # Errors
    /// Returns the [`UnderlayImageError`] from the decode: empty input, an
    /// oversized file, an unrecognized format, a claimed pixel count over
    /// the cap, or a malformed file the decoder rejected.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(&mut self, bytes: &[u8]) -> Result<(), UnderlayImageError> {
        match reticle_render::decode_underlay_image(bytes) {
            Ok(decoded) => {
                self.adopt_decoded(decoded);
                Ok(())
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                Err(e)
            }
        }
    }

    /// Adopts an already-decoded image (the wasm32 browser-decode result, or
    /// any other pre-decoded source), making it visible and clearing any
    /// prior load error. Unconditional (unlike [`UnderlayState::load`]),
    /// since decoding has already happened by the time this is called.
    pub fn adopt_decoded(&mut self, decoded: DecodedImage) {
        self.image = Some(decoded);
        self.visible = true;
        self.revision += 1;
        self.last_error = None;
    }

    /// Records a load failure's message (shown by the Inspector), leaving
    /// any previously loaded image untouched.
    pub fn record_load_error(&mut self, message: impl Into<String>) {
        self.last_error = Some(message.into());
    }

    /// The decoded image, if one has loaded successfully.
    #[must_use]
    pub fn image(&self) -> Option<&DecodedImage> {
        self.image.as_ref()
    }

    /// Whether an image has loaded (regardless of [`UnderlayState::visible`]).
    #[must_use]
    pub fn has_image(&self) -> bool {
        self.image.is_some()
    }

    /// The load revision: bumped on every successful load, so a texture
    /// cache keyed on this value only re-uploads when the pixels actually
    /// changed.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Clears the loaded image and hides the underlay, keeping the
    /// transform so loading a replacement image needs no re-alignment.
    pub fn clear(&mut self) {
        self.image = None;
        self.visible = false;
        self.revision += 1;
    }

    /// A cheap clone of the pending-pick mailbox, for the wasm async picker
    /// task to move in and post its result to.
    #[must_use]
    pub fn mailbox(&self) -> PickMailbox {
        self.pending.clone()
    }

    /// Drains the pending-pick mailbox, if the async browser picker has
    /// posted a result since the last drain (always `None` on native).
    pub fn take_pending_pick(&self) -> Option<PickResult> {
        self.pending.take()
    }

    /// Applies the `underlay.align` command: resets position to the world
    /// origin and scale to 1 world DBU per source pixel, a deterministic
    /// starting point the Inspector's position/scale fields then fine-tune
    /// (drag-to-place alignment is a follow-on; ADR 0118).
    pub fn align_to_origin(&mut self) {
        self.transform.set_position(0.0, 0.0);
        self.transform.set_scale(1.0);
    }

    /// Applies the `underlay.opacity` command: steps to the next preset in
    /// a fixed ladder (100% -> 75% -> 50% -> 25%, wrapping), landing on
    /// whichever preset is closest to (but strictly less than, or wrapping
    /// past) the current value. The Inspector's opacity slider gives
    /// continuous control; this gives the palette/menu a well-defined
    /// one-shot step.
    pub fn step_opacity_preset(&mut self) {
        const PRESETS: [f32; 4] = [1.0, 0.75, 0.5, 0.25];
        let current = self.transform.opacity;
        let next = PRESETS
            .iter()
            .copied()
            .find(|&p| p < current - f32::EPSILON)
            .unwrap_or(PRESETS[0]);
        self.transform.set_opacity(next);
    }
}

/// The largest encoded file [`decode_via_browser`] will hand to the browser,
/// in bytes (64 MiB, matching the native decode's identical cap): refuses a
/// hostile multi-gigabyte "image" before it is ever wrapped in a `Blob`.
#[cfg(target_arch = "wasm32")]
pub const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;

/// The largest decoded pixel count (`width * height`) [`decode_via_browser`]
/// will read back (64 megapixels, matching the native decode's identical
/// cap). Checked against `ImageBitmap`'s own dimensions, known before the
/// `getImageData` readback allocates the full RGBA buffer, so a decoded
/// image this large is rejected before that allocation rather than after.
#[cfg(target_arch = "wasm32")]
pub const MAX_DECODED_PIXELS: u64 = 64_000_000;

/// Decodes `bytes` through the browser's own image codec (wasm32 only):
/// builds a `Blob`, decodes it with `createImageBitmap` (the same decoder the
/// browser uses for `<img>`/CSS backgrounds, so PNG, JPEG, and anything else
/// the browser supports all work with no Rust-side format-specific code),
/// draws the result onto a detached canvas, and reads the pixels back with
/// `getImageData`. This is how the underlay avoids shipping a Rust image
/// decoder in the wasm bundle (ADR 0118).
///
/// Every fallible step returns a `String` error (there is no Rust-side byte
/// parsing here to panic on attacker-controlled input; a malformed image
/// simply rejects the `createImageBitmap` promise, which becomes an `Err`
/// here like any other failed browser call). The size caps below are the
/// untrusted-input discipline's equivalent of the native decode's
/// cap-before-allocate checks.
#[cfg(target_arch = "wasm32")]
pub async fn decode_via_browser(bytes: &[u8]) -> Result<DecodedImage, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap};

    if bytes.is_empty() {
        return Err("the image file is empty".to_owned());
    }
    if bytes.len() > MAX_ENCODED_BYTES {
        return Err(format!(
            "the image file is {} bytes, over the {MAX_ENCODED_BYTES}-byte limit",
            bytes.len()
        ));
    }

    let array = js_sys::Uint8Array::new_from_slice(bytes);
    let parts = js_sys::Array::of1(&array);
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)
        .map_err(|e| format!("could not build a blob from the image bytes: {e:?}"))?;

    let window = web_sys::window().ok_or_else(|| "no browser window".to_owned())?;
    let bitmap_promise = window
        .create_image_bitmap_with_blob(&blob)
        .map_err(|e| format!("the browser rejected the image: {e:?}"))?;
    let bitmap: ImageBitmap = JsFuture::from(bitmap_promise)
        .await
        .map_err(|e| format!("the browser could not decode the image: {e:?}"))?
        .dyn_into()
        .map_err(|_| "unexpected createImageBitmap result type".to_owned())?;

    let width = bitmap.width();
    let height = bitmap.height();
    let claimed_pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || claimed_pixels > MAX_DECODED_PIXELS {
        return Err(format!(
            "the image is {width}x{height}, over the {MAX_DECODED_PIXELS}-pixel limit"
        ));
    }

    let document = window
        .document()
        .ok_or_else(|| "no browser document".to_owned())?;
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|e| format!("could not create a canvas: {e:?}"))?
        .dyn_into()
        .map_err(|_| "unexpected element type".to_owned())?;
    canvas.set_width(width);
    canvas.set_height(height);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|e| format!("could not get a 2d canvas context: {e:?}"))?
        .ok_or_else(|| "no 2d canvas context".to_owned())?
        .dyn_into()
        .map_err(|_| "unexpected canvas context type".to_owned())?;
    ctx.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0)
        .map_err(|e| format!("could not draw the decoded image: {e:?}"))?;
    let image_data = ctx
        .get_image_data(0.0, 0.0, f64::from(width), f64::from(height))
        .map_err(|e| format!("could not read back the decoded pixels: {e:?}"))?;

    Ok(DecodedImage {
        width,
        height,
        rgba: image_data.data().0,
    })
}

#[cfg(test)]
mod tests {
    // Every float comparison below is against an exact literal or constant
    // the code under test assigns or clamps to directly (no accumulated
    // arithmetic), so it is exact in IEEE 754 binary32; matches the
    // established convention (`reticle_metrology::area`/`antenna`,
    // `crate::fps`, `crate::viewer`).
    #![allow(clippy::float_cmp)]

    use super::*;

    // Committed tiny (6x4) fixture rather than generated bytes: reticle-app's
    // own native-only `image` dependency (`crate::demoscript`) only enables
    // the `png` feature, so it cannot encode a JPEG itself, and this fixture
    // is shared with the JPEG test below (native decode uses `image`'s own
    // JPEG feature; `include_bytes!` resolves relative to this source file,
    // so it is independent of the test runner's working directory).
    #[cfg(not(target_arch = "wasm32"))]
    const TINY_PNG: &[u8] = include_bytes!("../tests/fixtures/underlay/tiny.png");
    #[cfg(not(target_arch = "wasm32"))]
    const TINY_JPEG: &[u8] = include_bytes!("../tests/fixtures/underlay/tiny.jpg");

    /// A small synthetic decoded image for tests that do not need a real
    /// file, just a plausible [`DecodedImage`] shape (every field is `pub`,
    /// so this needs no decoder at all).
    fn sample_decoded(width: u32, height: u32) -> DecodedImage {
        DecodedImage {
            width,
            height,
            rgba: vec![128_u8; (width * height * 4) as usize],
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn loads_a_png_fixture_into_state() {
        let mut state = UnderlayState::default();
        assert!(!state.has_image());
        state.load(TINY_PNG).expect("png loads");
        assert!(state.has_image());
        assert!(state.visible, "a fresh load becomes visible");
        let img = state.image().expect("image present");
        assert_eq!((img.width, img.height), (6, 4));
        assert_eq!(state.revision(), 1);
        assert!(state.last_error.is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn loads_a_jpeg_fixture_into_state_natively() {
        let mut state = UnderlayState::default();
        state.load(TINY_JPEG).expect("jpeg loads natively");
        let img = state.image().expect("image present");
        assert_eq!((img.width, img.height), (6, 4));
        assert_eq!(state.revision(), 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_load_keeps_the_previous_image_and_sets_last_error() {
        let mut state = UnderlayState::default();
        state.load(TINY_PNG).expect("first load succeeds");
        let rev_after_first = state.revision();

        let err = state.load(b"not an image").unwrap_err();
        assert_eq!(err, UnderlayImageError::UnrecognizedFormat);
        assert!(state.has_image(), "the earlier image is kept on failure");
        assert_eq!(state.revision(), rev_after_first, "no re-bump on failure");
        assert!(state.last_error.is_some());
    }

    #[test]
    fn adopt_decoded_makes_the_image_visible_and_bumps_revision() {
        let mut state = UnderlayState::default();
        assert!(!state.has_image());
        state.adopt_decoded(sample_decoded(6, 4));
        assert!(state.has_image());
        assert!(state.visible);
        assert_eq!(state.revision(), 1);
        assert!(state.last_error.is_none());
    }

    #[test]
    fn record_load_error_sets_the_message_without_touching_the_image() {
        let mut state = UnderlayState::default();
        state.adopt_decoded(sample_decoded(6, 4));
        let rev = state.revision();

        state.record_load_error("the browser could not decode the image");
        assert!(state.has_image(), "a later failure keeps the earlier image");
        assert_eq!(state.revision(), rev, "no re-bump on a recorded failure");
        assert_eq!(
            state.last_error.as_deref(),
            Some("the browser could not decode the image")
        );
    }

    #[test]
    fn clear_drops_the_image_and_hides_but_keeps_the_transform() {
        let mut state = UnderlayState::default();
        state.adopt_decoded(sample_decoded(6, 4));
        state.transform.set_position(120.0, -40.0);
        state.transform.set_scale(2.5);

        state.clear();
        assert!(!state.has_image());
        assert!(!state.visible);
        assert_eq!(state.transform.x, 120.0, "transform position preserved");
        assert_eq!(state.transform.scale, 2.5, "transform scale preserved");
    }

    #[test]
    fn set_position_and_scale_adjust_the_transform() {
        let mut t = UnderlayTransform::default();
        t.set_position(10.0, -5.0);
        t.set_scale(3.0);
        t.set_opacity(0.2);
        assert_eq!((t.x, t.y, t.scale, t.opacity), (10.0, -5.0, 3.0, 0.2));
    }

    #[test]
    fn scale_floors_at_min_scale_for_zero_negative_and_nan() {
        let mut t = UnderlayTransform::default();
        t.set_scale(0.0);
        assert_eq!(t.scale, MIN_SCALE);
        t.set_scale(-5.0);
        assert_eq!(t.scale, MIN_SCALE);
        t.set_scale(f32::NAN);
        assert_eq!(t.scale, MIN_SCALE);
    }

    #[test]
    fn opacity_clamps_into_unit_range() {
        let mut t = UnderlayTransform::default();
        t.set_opacity(-1.0);
        assert_eq!(t.opacity, 0.0);
        t.set_opacity(5.0);
        assert_eq!(t.opacity, 1.0);
        t.set_opacity(f32::NAN);
        assert_eq!(t.opacity, 0.0);
    }

    #[test]
    fn world_rect_places_bottom_left_at_position_and_scales_extent() {
        let mut t = UnderlayTransform::default();
        t.set_position(100.0, 200.0);
        t.set_scale(2.0);
        let r = t.world_rect(10, 5);
        assert_eq!(r.min, Point::new(100, 200));
        assert_eq!(r.max, Point::new(120, 210));
    }

    #[test]
    fn align_to_origin_resets_position_and_scale_only() {
        let mut state = UnderlayState::default();
        state.transform.set_position(55.0, -20.0);
        state.transform.set_scale(9.0);
        state.transform.set_opacity(0.3);

        state.align_to_origin();
        assert_eq!((state.transform.x, state.transform.y), (0.0, 0.0));
        assert_eq!(state.transform.scale, 1.0);
        assert_eq!(
            state.transform.opacity, 0.3,
            "opacity is untouched by align"
        );
    }

    #[test]
    fn step_opacity_preset_cycles_the_ladder_and_wraps() {
        let mut state = UnderlayState::default();
        state.transform.set_opacity(1.0);
        state.step_opacity_preset();
        assert_eq!(state.transform.opacity, 0.75);
        state.step_opacity_preset();
        assert_eq!(state.transform.opacity, 0.5);
        state.step_opacity_preset();
        assert_eq!(state.transform.opacity, 0.25);
        state.step_opacity_preset();
        assert_eq!(state.transform.opacity, 1.0, "wraps back to full");
    }

    #[test]
    fn step_opacity_preset_from_an_off_ladder_value_lands_on_the_next_lower_preset() {
        let mut state = UnderlayState::default();
        state.transform.set_opacity(0.9);
        state.step_opacity_preset();
        assert_eq!(state.transform.opacity, 0.75);
    }

    #[test]
    fn mailbox_round_trips_on_wasm_and_is_always_empty_on_native() {
        let state = UnderlayState::default();
        assert!(state.take_pending_pick().is_none());
        let mailbox = state.mailbox();
        mailbox.post(Ok(("tiny.png".to_owned(), sample_decoded(6, 4))));
        #[cfg(target_arch = "wasm32")]
        {
            let (name, image) = state.take_pending_pick().expect("posted").expect("ok");
            assert_eq!(name, "tiny.png");
            assert_eq!((image.width, image.height), (6, 4));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!(
                state.take_pending_pick().is_none(),
                "the mailbox is a no-op on native"
            );
        }
    }
}
