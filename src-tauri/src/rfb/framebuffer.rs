//! Client-side copy of the remote screen, stored as RGBA bytes.

use super::{Error, Result};

/// Largest framebuffer accepted (in pixels), to bound memory usage.
const MAX_PIXELS: usize = 16384 * 16384;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    pub fn area(&self) -> usize {
        self.w as usize * self.h as usize
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, other: &Rect) -> Rect {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x as u32 + self.w as u32).max(other.x as u32 + other.w as u32);
        let y1 = (self.y as u32 + self.h as u32).max(other.y as u32 + other.h as u32);
        Rect::new(x0, y0, (x1 - x0 as u32) as u16, (y1 - y0 as u32) as u16)
    }
}

pub struct Framebuffer {
    width: u16,
    height: u16,
    data: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u16, height: u16) -> Result<Self> {
        let mut fb = Self {
            width: 0,
            height: 0,
            data: Vec::new(),
        };
        fb.resize(width, height)?;
        Ok(fb)
    }

    /// Resizes the framebuffer; the content is reset to opaque black.
    pub fn resize(&mut self, width: u16, height: u16) -> Result<()> {
        let pixels = width as usize * height as usize;
        if pixels > MAX_PIXELS {
            return Err(Error::ScreenTooLarge(width, height));
        }
        self.width = width;
        self.height = height;
        self.data.clear();
        self.data
            .extend(std::iter::repeat_n([0, 0, 0, 255], pixels).flatten());
        Ok(())
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn contains(&self, r: Rect) -> bool {
        r.x as u32 + r.w as u32 <= self.width as u32
            && r.y as u32 + r.h as u32 <= self.height as u32
    }

    fn offset(&self, x: u16, y: u16) -> usize {
        (y as usize * self.width as usize + x as usize) * 4
    }

    fn row_mut(&mut self, r: Rect, row: u16) -> &mut [u8] {
        let start = self.offset(r.x, r.y + row);
        &mut self.data[start..start + r.w as usize * 4]
    }

    /// Fills `r` (which must be inside the framebuffer) with an RGBA colour.
    pub fn fill(&mut self, r: Rect, colour: [u8; 4]) {
        debug_assert!(self.contains(r));
        for row in 0..r.h {
            for px in self.row_mut(r, row).as_chunks_mut::<4>().0 {
                px.copy_from_slice(&colour);
            }
        }
    }

    /// Writes pixels given as `bytes_per_pixel` R,G,B[,X] bytes; alpha is forced opaque.
    pub fn put_pixels(&mut self, r: Rect, src: &[u8], bytes_per_pixel: usize) {
        debug_assert!(self.contains(r));
        debug_assert_eq!(src.len(), r.area() * bytes_per_pixel);
        let stride = r.w as usize * bytes_per_pixel;
        for row in 0..r.h {
            let src_row = &src[row as usize * stride..][..stride];
            for (dst, s) in self
                .row_mut(r, row)
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src_row.chunks_exact(bytes_per_pixel))
            {
                dst[..3].copy_from_slice(&s[..3]);
                dst[3] = 255;
            }
        }
    }

    /// Writes already-converted RGBA pixels.
    pub fn put_rgba(&mut self, r: Rect, src: &[u8]) {
        debug_assert!(self.contains(r));
        let stride = r.w as usize * 4;
        for row in 0..r.h {
            self.row_mut(r, row)
                .copy_from_slice(&src[row as usize * stride..][..stride]);
        }
    }

    /// CopyRect: copies the area at (`src_x`, `src_y`) to `dst`, handling overlap.
    pub fn copy(&mut self, src_x: u16, src_y: u16, dst: Rect) -> Result<()> {
        let src = Rect::new(src_x, src_y, dst.w, dst.h);
        if !self.contains(src) || !self.contains(dst) {
            return Err(Error::Malformed("CopyRect outside the screen".into()));
        }
        let len = dst.w as usize * 4;
        let mut copy_row = |row: u16| {
            let from = self.offset(src.x, src.y + row);
            let to = self.offset(dst.x, dst.y + row);
            self.data.copy_within(from..from + len, to);
        };
        if src_y < dst.y {
            (0..dst.h).rev().for_each(&mut copy_row);
        } else {
            (0..dst.h).for_each(&mut copy_row);
        }
        Ok(())
    }

    /// Appends the RGBA pixels of `r` to `out`.
    pub fn extract(&self, r: Rect, out: &mut Vec<u8>) {
        debug_assert!(self.contains(r));
        for row in 0..r.h {
            let start = self.offset(r.x, r.y + row);
            out.extend_from_slice(&self.data[start..start + r.w as usize * 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(fb: &Framebuffer, x: u16, y: u16) -> [u8; 4] {
        let o = fb.offset(x, y);
        fb.data[o..o + 4].try_into().unwrap()
    }

    #[test]
    fn overlapping_copy_downwards() {
        let mut fb = Framebuffer::new(4, 4).unwrap();
        for y in 0..4 {
            fb.fill(Rect::new(0, y, 4, 1), [y as u8, 0, 0, 255]);
        }
        fb.copy(0, 0, Rect::new(0, 1, 4, 3)).unwrap();
        let rows: Vec<u8> = (0..4).map(|y| pixel(&fb, 0, y)[0]).collect();
        assert_eq!(rows, [0, 0, 1, 2]);
    }

    #[test]
    fn copy_out_of_bounds_is_rejected() {
        let mut fb = Framebuffer::new(4, 4).unwrap();
        assert!(fb.copy(2, 2, Rect::new(0, 0, 3, 3)).is_err());
    }

    #[test]
    fn union_covers_both() {
        let u = Rect::new(2, 3, 4, 4).union(&Rect::new(10, 0, 2, 2));
        assert_eq!(u, Rect::new(2, 0, 10, 7));
    }
}
