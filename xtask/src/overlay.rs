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
