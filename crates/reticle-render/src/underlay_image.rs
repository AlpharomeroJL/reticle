//! Native decode for the image underlay (`reticle-app`'s `crate::underlay`,
//! ADR 0118): a positioned, scaled, opacity-controlled backdrop rendered
//! under the layout for tracing against a die photo or a datasheet figure.
//!
//! [`DecodedImage`] is the shared, unconditional result shape (plain pixel
//! data, no `image`-crate types), because BOTH targets produce one, by two
//! different routes:
//!
//! * Native: [`decode`], below, using the `image` crate (PNG and JPEG,
//!   `Cargo.toml`'s native-only dependency).
//! * wasm32: `reticle_app::underlay::decode_via_browser`, which decodes
//!   through the browser's own image codec (`createImageBitmap` plus a
//!   detached-canvas readback) instead of shipping a Rust decoder in the
//!   wasm bundle. A byte measurement (`just bundle-gate`) found that even
//!   `png`-feature-only added about 60 KiB gz here (`image` 0.25's
//!   unconditional `moxcms` color-management dependency ships regardless of
//!   which format feature is enabled), landing 43.4 KiB over the +450 KiB
//!   budget before JPEG was even considered; see the ADR for the full
//!   measurement and the decision.
//!
//! The bytes decoded here come from a file the user picked or dropped, so
//! they are untrusted input: [`decode`] caps both the encoded read
//! ([`MAX_ENCODED_BYTES`]) and the claimed decoded pixel count
//! ([`MAX_DECODED_PIXELS`]), checking the header-declared dimensions BEFORE
//! materializing the full pixel buffer, so a crafted header claiming an
//! enormous image is rejected with a structured error instead of an
//! unbounded allocation. Every failure path returns a `Result`; nothing here
//! calls `unwrap`/`expect`/indexing that could panic on attacker-controlled
//! bytes (the browser route has no Rust-side parsing at all: a malformed
//! image there just rejects the `createImageBitmap` promise).

/// A decoded underlay image: its pixel dimensions and tightly packed,
/// straight-alpha RGBA8 pixels (row 0 first, no padding), ready to hand to an
/// `egui` texture upload. Unconditional: both the native `image`-crate decode
/// below and the wasm32 browser decode (`reticle_app::underlay`) produce one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DecodedImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Straight-alpha RGBA8 pixels, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

// --- lane underlay: native decode via the `image` crate (ADR 0118) ---
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::DecodedImage;
    use std::fmt;
    use std::io::Cursor;

    /// The largest encoded file [`decode`] will read, in bytes (64 MiB). A
    /// real die photo or datasheet scan is a small fraction of this even
    /// uncompressed; the cap exists to refuse a hostile multi-gigabyte
    /// upload before it is ever handed to a decoder.
    pub const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;

    /// The largest decoded pixel count (`width * height`) [`decode`] will
    /// materialize (64 megapixels, e.g. up to 8192x8192). At 4 bytes/pixel
    /// this bounds the worst-case RGBA allocation to about 256 MiB, generous
    /// for a reference image yet far short of exhausting available memory.
    /// Checked against the header-declared size before the real decode runs,
    /// so a crafted header claiming more never reaches an allocation.
    pub const MAX_DECODED_PIXELS: u64 = 64_000_000;

    /// Why decoding an underlay image failed.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum UnderlayImageError {
        /// The input was empty.
        Empty,
        /// The encoded input exceeded [`MAX_ENCODED_BYTES`].
        TooLarge {
            /// The rejected input's length, in bytes.
            len: usize,
        },
        /// The bytes did not sniff as a supported image format.
        UnrecognizedFormat,
        /// The header-declared pixel count exceeded [`MAX_DECODED_PIXELS`],
        /// or declared a zero dimension.
        TooManyPixels {
            /// The header-declared width, in pixels.
            width: u32,
            /// The header-declared height, in pixels.
            height: u32,
        },
        /// The underlying decoder rejected the bytes as malformed; the
        /// message is the decoder's own description, not
        /// attacker-controlled formatting.
        Decode(String),
    }

    impl fmt::Display for UnderlayImageError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Empty => write!(f, "the image file is empty"),
                Self::TooLarge { len } => write!(
                    f,
                    "the image file is {len} bytes, over the {MAX_ENCODED_BYTES}-byte limit"
                ),
                Self::UnrecognizedFormat => {
                    write!(f, "the file is not a recognized PNG or JPEG image")
                }
                Self::TooManyPixels { width, height } => write!(
                    f,
                    "the image is {width}x{height}, over the {MAX_DECODED_PIXELS}-pixel limit"
                ),
                Self::Decode(msg) => write!(f, "could not decode the image: {msg}"),
            }
        }
    }

    impl std::error::Error for UnderlayImageError {}

    /// Sniffs the image format from its magic bytes without decoding any
    /// pixel data, or [`UnderlayImageError::UnrecognizedFormat`] if nothing
    /// matches.
    fn sniff_format(bytes: &[u8]) -> Result<image::ImageFormat, UnderlayImageError> {
        image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.format())
            .ok_or(UnderlayImageError::UnrecognizedFormat)
    }

    /// Decodes untrusted `bytes` (a PNG or JPEG) into straight-alpha RGBA8
    /// pixels.
    ///
    /// Untrusted-input discipline (checked in this order, each a `Result`
    /// return, never a panic):
    /// 1. Empty input is rejected.
    /// 2. The encoded length is capped at [`MAX_ENCODED_BYTES`] before
    ///    anything else touches the bytes.
    /// 3. The format is sniffed from magic bytes (no decode yet).
    /// 4. The header-declared dimensions are read (a cheap parse of just the
    ///    header, not the pixel data) and checked against
    ///    [`MAX_DECODED_PIXELS`] before the real decode runs, so a crafted
    ///    header claiming an enormous image is rejected before any large
    ///    allocation.
    /// 5. Only now does the real decode run, producing the full pixel
    ///    buffer.
    pub fn decode(bytes: &[u8]) -> Result<DecodedImage, UnderlayImageError> {
        if bytes.is_empty() {
            return Err(UnderlayImageError::Empty);
        }
        if bytes.len() > MAX_ENCODED_BYTES {
            return Err(UnderlayImageError::TooLarge { len: bytes.len() });
        }

        let format = sniff_format(bytes)?;

        let (width, height) = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| UnderlayImageError::Decode(e.to_string()))?
            .into_dimensions()
            .map_err(|e| UnderlayImageError::Decode(e.to_string()))?;
        let claimed_pixels = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || claimed_pixels > MAX_DECODED_PIXELS {
            return Err(UnderlayImageError::TooManyPixels { width, height });
        }

        let decoded = image::load_from_memory_with_format(bytes, format)
            .map_err(|e| UnderlayImageError::Decode(e.to_string()))?;
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(DecodedImage {
            width,
            height,
            rgba: rgba.into_raw(),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A tiny synthetic PNG (4x3, opaque red), built in-memory rather
        /// than committing a binary fixture for something this small.
        fn tiny_png_bytes() -> Vec<u8> {
            let img: image::RgbaImage =
                image::ImageBuffer::from_pixel(4, 3, image::Rgba([200, 30, 30, 255]));
            let mut out = Vec::new();
            img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
                .expect("encode tiny png");
            out
        }

        fn tiny_jpeg_bytes() -> Vec<u8> {
            let img: image::RgbImage =
                image::ImageBuffer::from_pixel(5, 2, image::Rgb([30, 120, 200]));
            let mut out = Vec::new();
            img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
                .expect("encode tiny jpeg");
            out
        }

        #[test]
        fn decodes_a_png() {
            let decoded = decode(&tiny_png_bytes()).expect("png decodes");
            assert_eq!((decoded.width, decoded.height), (4, 3));
            assert_eq!(decoded.rgba.len(), 4 * 3 * 4);
            // Every pixel is the fill color, straight alpha.
            assert_eq!(&decoded.rgba[0..4], &[200, 30, 30, 255]);
        }

        #[test]
        fn decodes_a_jpeg() {
            let decoded = decode(&tiny_jpeg_bytes()).expect("jpeg decodes");
            assert_eq!((decoded.width, decoded.height), (5, 2));
            assert_eq!(decoded.rgba.len(), 5 * 2 * 4);
        }

        #[test]
        fn empty_input_errors_not_panics() {
            assert_eq!(decode(&[]), Err(UnderlayImageError::Empty));
        }

        #[test]
        fn garbage_bytes_error_not_panic() {
            let garbage = vec![0x00_u8, 0x01, 0x02, 0x03, 0xff, 0xfe, 0x10, 0x20];
            assert_eq!(
                decode(&garbage),
                Err(UnderlayImageError::UnrecognizedFormat)
            );
        }

        #[test]
        fn truncated_png_header_errors_not_panics() {
            let full = tiny_png_bytes();
            // Keep only the PNG signature plus a sliver of the IHDR chunk:
            // enough to sniff as PNG, not enough to decode.
            let truncated = &full[..12.min(full.len())];
            let result = decode(truncated);
            assert!(result.is_err(), "truncated PNG must error, not panic");
        }

        #[test]
        fn oversized_input_is_rejected_before_any_decode() {
            let oversized = vec![0_u8; MAX_ENCODED_BYTES + 1];
            assert_eq!(
                decode(&oversized),
                Err(UnderlayImageError::TooLarge {
                    len: MAX_ENCODED_BYTES + 1
                })
            );
        }

        /// The standard (bit-by-bit, table-free) CRC-32 used by PNG chunks,
        /// so the crafted-header tests below can patch a chunk's trailing
        /// CRC to match its edited payload. Otherwise a CRC-checking decoder
        /// could reject the crafted chunk on the checksum before ever
        /// reaching the dimension check this test means to exercise.
        fn png_crc32(bytes: &[u8]) -> u32 {
            let mut crc: u32 = 0xFFFF_FFFF;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    let mask = 0_u32.wrapping_sub(crc & 1);
                    crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
                }
            }
            crc ^ 0xFFFF_FFFF
        }

        /// Overwrites the IHDR width/height in a well-formed PNG produced by
        /// [`tiny_png_bytes`] and patches the chunk's CRC to match, so the
        /// crafted header parses cleanly right up to (and only fails at)
        /// whatever check is under test.
        fn png_with_crafted_dimensions(width: u32, height: u32) -> Vec<u8> {
            let mut crafted = tiny_png_bytes();
            assert_eq!(&crafted[12..16], b"IHDR", "IHDR must be the first chunk");
            crafted[16..20].copy_from_slice(&width.to_be_bytes());
            crafted[20..24].copy_from_slice(&height.to_be_bytes());
            // Chunk type + payload (17 bytes: "IHDR" + 13-byte payload) is
            // what the CRC covers; the length field before it is not
            // included.
            let crc = png_crc32(&crafted[12..29]);
            crafted[29..33].copy_from_slice(&crc.to_be_bytes());
            crafted
        }

        /// A PNG whose IHDR claims a huge image but whose IDAT never
        /// supplies the pixel data: proves the dimension cap fires from the
        /// cheap header read BEFORE the real decode would try to allocate or
        /// inflate anything.
        #[test]
        fn crafted_huge_dimensions_are_rejected_before_allocating_pixels() {
            let huge: u32 = 60_000; // 3.6e9 px, over MAX_DECODED_PIXELS.
            let crafted = png_with_crafted_dimensions(huge, huge);

            let result = decode(&crafted);
            assert_eq!(
                result,
                Err(UnderlayImageError::TooManyPixels {
                    width: huge,
                    height: huge,
                }),
                "a crafted huge header must be capped before decoding pixels, got {result:?}"
            );
        }

        #[test]
        fn zero_dimension_is_rejected() {
            let crafted = png_with_crafted_dimensions(0, 3);
            // A zero width is invalid PNG (the spec requires both dimensions
            // positive), so a spec-conformant decoder may reject it itself
            // before this crate's own `TooManyPixels` cap gets a chance to;
            // either way it must be a structured error, never a panic or a
            // materialized image.
            assert!(
                decode(&crafted).is_err(),
                "a zero-width header must error, not panic or succeed"
            );
        }

        /// A broad sweep of small random/adversarial byte strings: none may
        /// ever panic. A panic inside `decode` fails the test itself (the
        /// harness reports it as a panicked test), which is the proof this
        /// needs; no `catch_unwind` machinery is required to observe it.
        #[test]
        fn random_byte_strings_never_panic() {
            // A tiny xorshift so this has no extra dev-dependency:
            // deterministic, fast, and plenty for an adversarial
            // byte-string sweep.
            let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };
            for _ in 0..500 {
                let len = (next() % 300) as usize;
                let bytes: Vec<u8> = (0..len).map(|_| (next() % 256) as u8).collect();
                let _ = decode(&bytes);
            }
        }
    }
}
// --- end lane underlay ---

#[cfg(not(target_arch = "wasm32"))]
pub use native::{MAX_DECODED_PIXELS, MAX_ENCODED_BYTES, UnderlayImageError, decode};
