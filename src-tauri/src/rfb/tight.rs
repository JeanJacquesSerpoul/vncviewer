//! Tight encoding (7): fill, JPEG, and zlib-compressed "basic" rectangles with
//! the copy, palette and gradient filters.
//!
//! With the RGBX pixel format the client requests, a Tight pixel ("TPIXEL")
//! is 3 bytes: R, G, B.

use std::io::Cursor;

use flate2::Decompress;
use tokio::io::{AsyncRead, AsyncReadExt};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use super::decode::inflate;
use super::framebuffer::{Framebuffer, Rect};
use super::{Error, Result};

const FILL: u8 = 0x08;
const JPEG: u8 = 0x09;
const EXPLICIT_FILTER: u8 = 0x04;
const FILTER_COPY: u8 = 0;
const FILTER_PALETTE: u8 = 1;
const FILTER_GRADIENT: u8 = 2;
/// Data shorter than this is sent without compression.
const MIN_TO_COMPRESS: usize = 12;
const MAX_DATA: usize = 64 << 20;

pub struct Tight {
    /// Tight uses four persistent zlib streams, chosen per rectangle.
    zlib: [Decompress; 4],
    compressed: Vec<u8>,
    data: Vec<u8>,
    rgba: Vec<u8>,
}

impl Default for Tight {
    fn default() -> Self {
        Self {
            zlib: std::array::from_fn(|_| Decompress::new(true)),
            compressed: Vec::new(),
            data: Vec::new(),
            rgba: Vec::new(),
        }
    }
}

impl Tight {
    pub async fn read_rect<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        fb: &mut Framebuffer,
        rect: Rect,
    ) -> Result<()> {
        let control = r.read_u8().await?;
        for (i, z) in self.zlib.iter_mut().enumerate() {
            if control & (1 << i) != 0 {
                z.reset(true);
            }
        }
        let kind = control >> 4;
        if kind == FILL {
            let mut p = [0u8; 3];
            r.read_exact(&mut p).await?;
            fb.fill(rect, [p[0], p[1], p[2], 255]);
            return Ok(());
        }
        if kind == JPEG {
            let len = read_compact_len(r).await?;
            read_vec(r, &mut self.compressed, len).await?;
            return self.put_jpeg(fb, rect);
        }
        if kind > JPEG {
            return Err(Error::Malformed(format!(
                "invalid Tight control {control:#x}"
            )));
        }

        let stream = (kind & 3) as usize;
        let filter = if kind & EXPLICIT_FILTER != 0 {
            r.read_u8().await?
        } else {
            FILTER_COPY
        };
        let (w, h) = (rect.w as usize, rect.h as usize);
        let mut palette = Vec::new();
        let size = match filter {
            FILTER_COPY | FILTER_GRADIENT => w * h * 3,
            FILTER_PALETTE => {
                let n = r.read_u8().await? as usize + 1;
                let mut p = vec![0u8; n * 3];
                r.read_exact(&mut p).await?;
                palette = p
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| [c[0], c[1], c[2], 255])
                    .collect();
                if n == 2 { w.div_ceil(8) * h } else { w * h }
            }
            f => return Err(Error::Malformed(format!("invalid Tight filter {f}"))),
        };

        if size < MIN_TO_COMPRESS {
            read_vec(r, &mut self.data, size).await?;
        } else {
            let len = read_compact_len(r).await?;
            read_vec(r, &mut self.compressed, len).await?;
            inflate(&mut self.zlib[stream], &self.compressed, &mut self.data)?;
            if self.data.len() < size {
                return Err(Error::Malformed("truncated Tight data".into()));
            }
            self.data.truncate(size);
        }

        match filter {
            FILTER_COPY => fb.put_pixels(rect, &self.data, 3),
            FILTER_GRADIENT => {
                ungradient(&mut self.data, w);
                fb.put_pixels(rect, &self.data, 3);
            }
            _ => {
                self.rgba.clear();
                if palette.len() == 2 {
                    for row in self.data.chunks_exact(w.div_ceil(8)) {
                        for x in 0..w {
                            let bit = (row[x / 8] >> (7 - x % 8)) & 1;
                            self.rgba.extend_from_slice(&palette[bit as usize]);
                        }
                    }
                } else {
                    for &i in &self.data {
                        let c = palette
                            .get(i as usize)
                            .ok_or_else(|| Error::Malformed("invalid palette index".into()))?;
                        self.rgba.extend_from_slice(c);
                    }
                }
                fb.put_rgba(rect, &self.rgba);
            }
        }
        Ok(())
    }

    fn put_jpeg(&mut self, fb: &mut Framebuffer, rect: Rect) -> Result<()> {
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut decoder =
            JpegDecoder::new_with_options(Cursor::new(self.compressed.as_slice()), options);
        let pixels = decoder
            .decode()
            .map_err(|e| Error::Malformed(format!("invalid JPEG: {e:?}")))?;
        let info = decoder
            .info()
            .ok_or_else(|| Error::Malformed("invalid JPEG".into()))?;
        if (info.width, info.height) != (rect.w, rect.h) || pixels.len() != rect.area() * 4 {
            return Err(Error::Malformed(format!(
                "JPEG is {}×{}, rectangle {}×{}",
                info.width, info.height, rect.w, rect.h
            )));
        }
        fb.put_rgba(rect, &pixels);
        Ok(())
    }
}

/// Reads Tight's 1–3 byte "compact" length.
async fn read_compact_len<R: AsyncRead + Unpin>(r: &mut R) -> Result<usize> {
    let mut len = 0usize;
    for i in 0..3 {
        let b = r.read_u8().await?;
        if i == 2 {
            len |= (b as usize) << 14;
            break;
        }
        len |= ((b & 0x7f) as usize) << (7 * i);
        if b & 0x80 == 0 {
            break;
        }
    }
    if len > MAX_DATA {
        return Err(Error::Malformed(format!(
            "Tight data too large ({len} bytes)"
        )));
    }
    Ok(len)
}

async fn read_vec<R: AsyncRead + Unpin>(r: &mut R, buf: &mut Vec<u8>, len: usize) -> Result<()> {
    buf.resize(len, 0);
    r.read_exact(buf).await?;
    Ok(())
}

/// Undoes the gradient filter in place: each byte was stored as the difference
/// from the prediction `left + above - above_left`, clamped to 0..=255.
fn ungradient(data: &mut [u8], w: usize) {
    let stride = w * 3;
    for i in 0..data.len() {
        let (x, y) = ((i % stride) / 3, i / stride);
        let left = if x > 0 { data[i - 3] as i32 } else { 0 };
        let above = if y > 0 { data[i - stride] as i32 } else { 0 };
        let above_left = if x > 0 && y > 0 {
            data[i - stride - 3] as i32
        } else {
            0
        };
        let predicted = (left + above - above_left).clamp(0, 255) as u8;
        data[i] = data[i].wrapping_add(predicted);
    }
}

#[cfg(test)]
mod tests {
    use flate2::{Compress, Compression, FlushCompress};

    use super::*;

    fn compact(len: usize) -> Vec<u8> {
        match len {
            0..0x80 => vec![len as u8],
            0x80..0x4000 => vec![(len as u8) | 0x80, (len >> 7) as u8],
            _ => vec![
                (len as u8) | 0x80,
                ((len >> 7) as u8) | 0x80,
                (len >> 14) as u8,
            ],
        }
    }

    fn zlib(z: &mut Compress, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 64);
        z.compress_vec(data, &mut out, FlushCompress::Sync).unwrap();
        out
    }

    fn pixel(fb: &Framebuffer, x: u16, y: u16) -> [u8; 4] {
        let mut out = Vec::new();
        fb.extract(Rect::new(x, y, 1, 1), &mut out);
        out.try_into().unwrap()
    }

    #[tokio::test]
    async fn compact_lengths() {
        for len in [0, 5, 127, 128, 300, 16383, 16384, 1_000_000] {
            let bytes = compact(len);
            assert_eq!(read_compact_len(&mut bytes.as_slice()).await.unwrap(), len);
        }
    }

    #[tokio::test]
    async fn fill_palette_copy_and_gradient() {
        let mut fb = Framebuffer::new(16, 16).unwrap();
        let mut t = Tight::default();
        let mut z = [0, 1, 2].map(|_| Compress::new(Compression::default(), true));
        let mut wire = Vec::new();

        // Fill (0,0,16,16) grey.
        wire.extend([FILL << 4, 50, 60, 70]);
        // Two-colour palette (0,0,10,2), stream 1: alternate columns.
        let mut bits = Vec::new();
        for _ in 0..2 {
            bits.extend([0b1010_1010, 0b1000_0000]);
        }
        wire.extend([
            (EXPLICIT_FILTER | 1) << 4,
            FILTER_PALETTE,
            1,
            255,
            0,
            0,
            0,
            0,
            255,
        ]);
        wire.extend(bits); // 4 bytes < 12: not compressed
        // Copy (0,4,4,4), stream 0, compressed.
        let raw: Vec<u8> = (0..16).flat_map(|i| [i as u8 * 10, 1, 2]).collect();
        let c = zlib(&mut z[0], &raw);
        wire.push(0);
        wire.extend(compact(c.len()));
        wire.extend(c);
        // Gradient (8,8,4,4), stream 2: a constant image encodes as one value then zeros.
        let mut grad = vec![0u8; 4 * 4 * 3];
        grad[..3].copy_from_slice(&[9, 8, 7]);
        // Row 0: left neighbour predicts; column 0: above predicts; inside: left+above-al.
        let c = zlib(&mut z[2], &grad);
        wire.extend([(EXPLICIT_FILTER | 2) << 4, FILTER_GRADIENT]);
        wire.extend(compact(c.len()));
        wire.extend(c);

        let mut r = wire.as_slice();
        t.read_rect(&mut r, &mut fb, Rect::new(0, 0, 16, 16))
            .await
            .unwrap();
        t.read_rect(&mut r, &mut fb, Rect::new(0, 0, 10, 2))
            .await
            .unwrap();
        t.read_rect(&mut r, &mut fb, Rect::new(0, 4, 4, 4))
            .await
            .unwrap();
        t.read_rect(&mut r, &mut fb, Rect::new(8, 8, 4, 4))
            .await
            .unwrap();
        assert!(r.is_empty());

        assert_eq!(pixel(&fb, 15, 15), [50, 60, 70, 255]);
        assert_eq!(pixel(&fb, 0, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&fb, 1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(&fb, 8, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&fb, 3, 7), [150, 1, 2, 255]);
        // Pixel (0,0) of the gradient rect is the raw value; the others predict it,
        // except that row 0 / column 0 predict from a single neighbour.
        assert_eq!(pixel(&fb, 8, 8), [9, 8, 7, 255]);
        assert_eq!(pixel(&fb, 11, 8), [9, 8, 7, 255]);
        assert_eq!(pixel(&fb, 8, 11), [9, 8, 7, 255]);
        assert_eq!(pixel(&fb, 11, 11), [9, 8, 7, 255]);
    }

    #[test]
    fn gradient_prediction_is_clamped() {
        // 2x2 one channel per pixel shown as RGB triples with equal channels.
        let px = |v: u8| [v, v, v];
        // Image: [200, 250] / [250, ?]; prediction for (1,1) = 250+250-200 = 300 -> 255.
        let mut data: Vec<u8> = [px(200), px(50), px(50), px(3)].concat();
        ungradient(&mut data, 2);
        assert_eq!(&data[..3], &[200; 3]);
        assert_eq!(&data[3..6], &[250; 3]);
        assert_eq!(&data[6..9], &[250; 3]);
        assert_eq!(&data[9..], &[2; 3]); // 255 + 3 wraps to 2
    }
}
