//! CPU overlay drawing on offscreen RGBA renders: DRC markers, minimap chrome,
//! and presence cursors. Pixel coordinates have row 0 at the top; colors are
//! straight (non-premultiplied) RGBA bytes alpha-blended over the frame.

use reticle_geometry::{Point, Rect};
use reticle_model::Camera;

/// A mutable RGBA8 pixel canvas over a rendered frame.
pub struct Canvas<'a> {
    buf: &'a mut [u8],
    width: i32,
    height: i32,
}

impl<'a> Canvas<'a> {
    /// Wraps a tightly packed RGBA buffer of `size` pixels.
    ///
    /// # Panics
    ///
    /// Panics if the buffer length does not match `size`.
    pub fn new(buf: &'a mut [u8], size: (u32, u32)) -> Self {
        assert_eq!(buf.len(), (size.0 * size.1 * 4) as usize);
        Self {
            buf,
            width: size.0 as i32,
            height: size.1 as i32,
        }
    }

    /// Alpha-blends one pixel; out-of-bounds coordinates are ignored.
    pub fn blend(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        let a = f32::from(color[3]) / 255.0;
        for (dst, &src) in self.buf[i..i + 3].iter_mut().zip(&color[..3]) {
            *dst = (f32::from(src) * a + f32::from(*dst) * (1.0 - a)).round() as u8;
        }
        self.buf[i + 3] = 255;
    }

    /// Fills the axis-aligned pixel rectangle `[x0, x1) x [y0, y1)`.
    pub fn fill_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 4]) {
        let (x0, x1) = (x0.round() as i32, x1.round() as i32);
        let (y0, y1) = (y0.round() as i32, y1.round() as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend(x, y, color);
            }
        }
    }

    /// Copies an opaque RGBA sub-image into the canvas with its top-left corner
    /// at `(dst_x, dst_y)`, clipping at the canvas edges.
    pub fn blit(&mut self, src: &[u8], src_size: (u32, u32), dst_x: i32, dst_y: i32) {
        for sy in 0..src_size.1 as i32 {
            for sx in 0..src_size.0 as i32 {
                let (x, y) = (dst_x + sx, dst_y + sy);
                if x < 0 || y < 0 || x >= self.width || y >= self.height {
                    continue;
                }
                let si = ((sy * src_size.0 as i32 + sx) * 4) as usize;
                let di = ((y * self.width + x) * 4) as usize;
                self.buf[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }

    /// Strokes the rectangle border with a band of `thickness` pixels drawn
    /// outward from the given edges.
    pub fn stroke_rect(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        thickness: f32,
        color: [u8; 4],
    ) {
        let t = thickness.max(1.0);
        self.fill_rect(x0 - t, y0 - t, x1 + t, y0, color);
        self.fill_rect(x0 - t, y1, x1 + t, y1 + t, color);
        self.fill_rect(x0 - t, y0, x0, y1, color);
        self.fill_rect(x1, y0, x1 + t, y1, color);
    }
}

impl Canvas<'_> {
    /// Fills the triangle `a`-`b`-`c` (pixel coordinates, any winding).
    pub fn fill_tri(&mut self, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: [u8; 4]) {
        let edge = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| {
            (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
        };
        let area = edge(a, b, c);
        if area == 0.0 {
            return;
        }
        let x0 = a.0.min(b.0).min(c.0).floor() as i32;
        let x1 = a.0.max(b.0).max(c.0).ceil() as i32;
        let y0 = a.1.min(b.1).min(c.1).floor() as i32;
        let y1 = a.1.max(b.1).max(c.1).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = (x as f32 + 0.5, y as f32 + 0.5);
                let (w0, w1, w2) = (edge(a, b, p), edge(b, c, p), edge(c, a, p));
                let inside = if area > 0.0 {
                    w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
                } else {
                    w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
                };
                if inside {
                    self.blend(x, y, color);
                }
            }
        }
    }

    /// Draws a 5x7 bitmap letter with its top-left corner at `(x, y)`, magnified
    /// by `scale`. Letters without a bitmap are skipped.
    pub fn draw_glyph(&mut self, letter: char, x: f32, y: f32, scale: i32, color: [u8; 4]) {
        let Some(rows) = glyph5x7(letter) else {
            return;
        };
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5i32 {
                if bits & (0b1_0000 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            self.blend(ox + col * scale + dx, oy + row as i32 * scale + dy, color);
                        }
                    }
                }
            }
        }
    }
}

/// The 5x7 bitmap for one letter (top row first, bit 4 = leftmost column).
/// Only the letters the presence chips need are defined.
fn glyph5x7(letter: char) -> Option<[u8; 7]> {
    match letter {
        'A' => Some([
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ]),
        'G' => Some([
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ]),
        _ => None,
    }
}

/// Maps world coordinates (DBU, `+y` up) into frame pixels (`+y` down) for a
/// camera, mirroring the transform the 2D render pass applies.
pub struct WorldMap {
    center: Point,
    ppd: f32,
    half_w: f32,
    half_h: f32,
}

impl WorldMap {
    /// Builds the map for `camera` rendering into `size` pixels.
    pub fn new(camera: &Camera, size: (u32, u32)) -> Self {
        Self {
            center: camera.center,
            ppd: camera.pixels_per_dbu,
            half_w: size.0 as f32 / 2.0,
            half_h: size.1 as f32 / 2.0,
        }
    }

    /// The pixel position of a world point.
    pub fn to_px(&self, p: Point) -> (f32, f32) {
        let x = (p.x - self.center.x) as f32 * self.ppd + self.half_w;
        let y = (self.center.y - p.y) as f32 * self.ppd + self.half_h;
        (x, y)
    }

    /// The pixel rectangle `(left, top, right, bottom)` of a world rectangle.
    pub fn rect_to_px(&self, r: Rect) -> (f32, f32, f32, f32) {
        let (left, bottom) = self.to_px(r.min);
        let (right, top) = self.to_px(r.max);
        (left, top, right, bottom)
    }
}
