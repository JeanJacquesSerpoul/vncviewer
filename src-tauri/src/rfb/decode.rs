//! Server-to-client messages and rectangle decoders.
//!
//! The client always requests [`PixelFormat::RGBX`](super::proto::PixelFormat::RGBX),
//! so a pixel is 4 bytes (R, G, B, X) and a ZRLE "CPIXEL" is 3 bytes (R, G, B).

use flate2::{Decompress, FlushDecompress};
use tokio::io::{AsyncRead, AsyncReadExt};

use super::framebuffer::{Framebuffer, Rect};
use super::proto::{encoding, latin1_decode};
use super::tight::Tight;
use super::{Error, Result, clipboard};

const MAX_CUT_TEXT: usize = 16 << 20;
const MAX_ZRLE_DATA: usize = 64 << 20;
const MAX_CURSOR_SIDE: u16 = 1024;

#[derive(Debug)]
pub enum ServerMessage {
    FramebufferUpdate(Update),
    SetColourMap,
    Bell,
    CutText(String),
    /// Extended clipboard message.
    Clipboard(clipboard::Message),
}

/// Result of one FramebufferUpdate.
#[derive(Debug, Default)]
pub struct Update {
    /// Areas of the framebuffer whose content changed.
    pub dirty: Vec<Rect>,
    /// The framebuffer was resized (DesktopSize pseudo-encoding).
    pub resized: bool,
    /// New cursor shape (Cursor pseudo-encoding).
    pub cursor: Option<CursorShape>,
    /// ExtendedDesktopSize was received: the server can resize on request.
    pub desktop: Option<DesktopLayout>,
}

/// Screen layout reported by ExtendedDesktopSize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopLayout {
    /// Id of the first screen, to reuse when asking for a new size.
    pub screen_id: u32,
    /// Why it was sent: 0 server-side change, 1 our request, 2 another client.
    pub reason: u16,
    /// 0 on success; otherwise our resize request was refused.
    pub status: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorShape {
    pub hot_x: u16,
    pub hot_y: u16,
    pub width: u16,
    pub height: u16,
    /// RGBA pixels, transparent where the mask is clear.
    pub rgba: Vec<u8>,
}

pub struct Decoder {
    /// ZRLE uses a single zlib stream for the whole connection.
    zlib: Decompress,
    inflated: Vec<u8>,
    scratch: Vec<u8>,
    tight: Tight,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            zlib: Decompress::new(true),
            inflated: Vec::new(),
            scratch: Vec::new(),
            tight: Tight::default(),
        }
    }

    /// Reads one server message, applying framebuffer updates to `fb`.
    pub async fn read_message<R>(
        &mut self,
        r: &mut R,
        fb: &mut Framebuffer,
    ) -> Result<ServerMessage>
    where
        R: AsyncRead + Unpin,
    {
        match r.read_u8().await? {
            0 => {
                r.read_u8().await?;
                let count = r.read_u16().await?;
                self.read_update(r, fb, count)
                    .await
                    .map(ServerMessage::FramebufferUpdate)
            }
            1 => {
                r.read_u8().await?;
                r.read_u16().await?; // first colour
                let n = r.read_u16().await?;
                self.read_into_scratch(r, n as usize * 6).await?;
                Ok(ServerMessage::SetColourMap)
            }
            2 => Ok(ServerMessage::Bell),
            3 => {
                let mut pad = [0u8; 3];
                r.read_exact(&mut pad).await?;
                // A negative length marks an extended clipboard message.
                let len = r.read_i32().await?;
                let size = len.unsigned_abs() as usize;
                if size > MAX_CUT_TEXT {
                    return Err(Error::Malformed(format!(
                        "clipboard text too large ({size} bytes)"
                    )));
                }
                self.read_into_scratch(r, size).await?;
                if len < 0 {
                    clipboard::parse(&self.scratch).map(ServerMessage::Clipboard)
                } else {
                    Ok(ServerMessage::CutText(latin1_decode(&self.scratch)))
                }
            }
            t => Err(Error::UnknownMessage(t)),
        }
    }

    async fn read_update<R>(
        &mut self,
        r: &mut R,
        fb: &mut Framebuffer,
        count: u16,
    ) -> Result<Update>
    where
        R: AsyncRead + Unpin,
    {
        let mut update = Update::default();
        for _ in 0..count {
            let rect = Rect::new(
                r.read_u16().await?,
                r.read_u16().await?,
                r.read_u16().await?,
                r.read_u16().await?,
            );
            let enc = r.read_i32().await?;
            match enc {
                encoding::LAST_RECT => break,
                encoding::DESKTOP_SIZE => {
                    fb.resize(rect.w, rect.h)?;
                    update.resized = true;
                    update.dirty.clear();
                    continue;
                }
                encoding::CURSOR => {
                    update.cursor = Some(self.read_cursor(r, rect).await?);
                    continue;
                }
                encoding::EXTENDED_DESKTOP_SIZE => {
                    let layout = self.read_desktop_layout(r, rect).await?;
                    if (rect.w, rect.h) != (fb.width(), fb.height()) && layout.status == 0 {
                        fb.resize(rect.w, rect.h)?;
                        update.resized = true;
                        update.dirty.clear();
                    }
                    update.desktop = Some(layout);
                    continue;
                }
                _ => {}
            }
            if !fb.contains(rect) {
                return Err(Error::Malformed(format!(
                    "rectangle outside the screen: {rect:?}"
                )));
            }
            match enc {
                encoding::RAW => {
                    self.read_into_scratch(r, rect.area() * 4).await?;
                    fb.put_pixels(rect, &self.scratch, 4);
                }
                encoding::COPY_RECT => {
                    let (sx, sy) = (r.read_u16().await?, r.read_u16().await?);
                    fb.copy(sx, sy, rect)?;
                }
                encoding::RRE => self.read_rre(r, fb, rect).await?,
                encoding::HEXTILE => self.read_hextile(r, fb, rect).await?,
                encoding::ZRLE => self.read_zrle(r, fb, rect).await?,
                encoding::TIGHT => self.tight.read_rect(r, fb, rect).await?,
                e => return Err(Error::UnsupportedEncoding(e)),
            }
            if !rect.is_empty() {
                update.dirty.push(rect);
            }
        }
        Ok(update)
    }

    async fn read_into_scratch<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        len: usize,
    ) -> Result<()> {
        self.scratch.resize(len, 0);
        r.read_exact(&mut self.scratch).await?;
        Ok(())
    }

    async fn read_rre<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        fb: &mut Framebuffer,
        rect: Rect,
    ) -> Result<()> {
        let count = r.read_u32().await? as usize;
        let bg = read_pixel(r).await?;
        fb.fill(rect, bg);
        // Read subrectangles in batches to keep the scratch buffer bounded.
        let mut left = count;
        while left > 0 {
            let n = left.min(4096);
            self.read_into_scratch(r, n * 12).await?;
            for s in self.scratch.as_chunks::<12>().0 {
                let colour = [s[0], s[1], s[2], 255];
                let sub = Rect::new(be16(&s[4..]), be16(&s[6..]), be16(&s[8..]), be16(&s[10..]));
                fb.fill(sub_rect(rect, sub)?, colour);
            }
            left -= n;
        }
        Ok(())
    }

    async fn read_hextile<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        fb: &mut Framebuffer,
        rect: Rect,
    ) -> Result<()> {
        const RAW: u8 = 1;
        const BACKGROUND: u8 = 2;
        const FOREGROUND: u8 = 4;
        const ANY_SUBRECTS: u8 = 8;
        const COLOURED: u8 = 16;

        let mut bg = [0, 0, 0, 255];
        let mut fg = [0, 0, 0, 255];
        for ty in (0..rect.h).step_by(16) {
            for tx in (0..rect.w).step_by(16) {
                let tile = Rect::new(
                    rect.x + tx,
                    rect.y + ty,
                    16.min(rect.w - tx),
                    16.min(rect.h - ty),
                );
                let flags = r.read_u8().await?;
                if flags & RAW != 0 {
                    self.read_into_scratch(r, tile.area() * 4).await?;
                    fb.put_pixels(tile, &self.scratch, 4);
                    continue;
                }
                if flags & BACKGROUND != 0 {
                    bg = read_pixel(r).await?;
                }
                fb.fill(tile, bg);
                if flags & FOREGROUND != 0 {
                    fg = read_pixel(r).await?;
                }
                if flags & ANY_SUBRECTS == 0 {
                    continue;
                }
                let count = r.read_u8().await? as usize;
                let coloured = flags & COLOURED != 0;
                let size = if coloured { 6 } else { 2 };
                self.read_into_scratch(r, count * size).await?;
                for s in self.scratch.chunks_exact(size) {
                    let (colour, xy, wh) = if coloured {
                        ([s[0], s[1], s[2], 255], s[4], s[5])
                    } else {
                        (fg, s[0], s[1])
                    };
                    let sub = Rect::new(
                        (xy >> 4) as u16,
                        (xy & 15) as u16,
                        (wh >> 4) as u16 + 1,
                        (wh & 15) as u16 + 1,
                    );
                    fb.fill(sub_rect(tile, sub)?, colour);
                }
            }
        }
        Ok(())
    }

    async fn read_zrle<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        fb: &mut Framebuffer,
        rect: Rect,
    ) -> Result<()> {
        let len = r.read_u32().await? as usize;
        if len > MAX_ZRLE_DATA {
            return Err(Error::Malformed(format!(
                "ZRLE data too large ({len} bytes)"
            )));
        }
        self.read_into_scratch(r, len).await?;
        inflate(&mut self.zlib, &self.scratch, &mut self.inflated)?;

        let mut data = ByteReader {
            buf: &self.inflated,
            pos: 0,
        };
        let mut tile_buf = Vec::with_capacity(64 * 64 * 4);
        for ty in (0..rect.h).step_by(64) {
            for tx in (0..rect.w).step_by(64) {
                let tile = Rect::new(
                    rect.x + tx,
                    rect.y + ty,
                    64.min(rect.w - tx),
                    64.min(rect.h - ty),
                );
                decode_zrle_tile(&mut data, tile, &mut tile_buf)?;
                fb.put_rgba(tile, &tile_buf);
            }
        }
        Ok(())
    }

    /// ExtendedDesktopSize: x is the reason, y the status; then the screens.
    async fn read_desktop_layout<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        rect: Rect,
    ) -> Result<DesktopLayout> {
        let screens = r.read_u8().await? as usize;
        self.read_into_scratch(r, 3 + screens * 16).await?;
        let screen_id = self
            .scratch
            .get(3..7)
            .map_or(0, |b| u32::from_be_bytes(b.try_into().unwrap()));
        Ok(DesktopLayout {
            screen_id,
            reason: rect.x,
            status: rect.y,
        })
    }

    async fn read_cursor<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        rect: Rect,
    ) -> Result<CursorShape> {
        if rect.w > MAX_CURSOR_SIDE || rect.h > MAX_CURSOR_SIDE {
            return Err(Error::Malformed(format!(
                "cursor too large ({}×{})",
                rect.w, rect.h
            )));
        }
        let mask_stride = (rect.w as usize).div_ceil(8);
        self.read_into_scratch(r, rect.area() * 4 + mask_stride * rect.h as usize)
            .await?;
        let (pixels, mask) = self.scratch.split_at(rect.area() * 4);
        let mut rgba = Vec::with_capacity(rect.area() * 4);
        for y in 0..rect.h as usize {
            for x in 0..rect.w as usize {
                let p = &pixels[(y * rect.w as usize + x) * 4..][..3];
                let visible = mask[y * mask_stride + x / 8] & (0x80 >> (x % 8)) != 0;
                rgba.extend_from_slice(&[p[0], p[1], p[2], if visible { 255 } else { 0 }]);
            }
        }
        Ok(CursorShape {
            hot_x: rect.x,
            hot_y: rect.y,
            width: rect.w,
            height: rect.h,
            rgba,
        })
    }
}

async fn read_pixel<R: AsyncRead + Unpin>(r: &mut R) -> Result<[u8; 4]> {
    let mut p = [0u8; 4];
    r.read_exact(&mut p).await?;
    Ok([p[0], p[1], p[2], 255])
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

/// Converts a subrectangle relative to `parent` into absolute coordinates.
fn sub_rect(parent: Rect, sub: Rect) -> Result<Rect> {
    if sub.x as u32 + sub.w as u32 > parent.w as u32
        || sub.y as u32 + sub.h as u32 > parent.h as u32
    {
        return Err(Error::Malformed(format!(
            "subrectangle out of bounds: {sub:?}"
        )));
    }
    Ok(Rect::new(parent.x + sub.x, parent.y + sub.y, sub.w, sub.h))
}

/// Feeds all of `input` to the persistent zlib stream; output goes to `out`.
pub(super) fn inflate(z: &mut Decompress, input: &[u8], out: &mut Vec<u8>) -> Result<()> {
    out.clear();
    let mut consumed = 0;
    loop {
        if out.capacity() - out.len() < 32 * 1024 {
            out.reserve(out.capacity().max(64 * 1024));
        }
        let (in_before, out_before) = (z.total_in(), z.total_out());
        z.decompress_vec(&input[consumed..], out, FlushDecompress::Sync)
            .map_err(|e| Error::Malformed(format!("invalid zlib stream: {e}")))?;
        consumed += (z.total_in() - in_before) as usize;
        let produced = z.total_out() - out_before;
        let output_full = out.len() == out.capacity();
        if consumed >= input.len() && !output_full {
            return Ok(());
        }
        if produced == 0 && z.total_in() == in_before && !output_full {
            return Err(Error::Malformed("truncated zlib stream".into()));
        }
    }
}

struct ByteReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self
            .buf
            .get(self.pos..self.pos + n)
            .ok_or_else(|| Error::Malformed("truncated ZRLE data".into()))?;
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn cpixel(&mut self) -> Result<[u8; 4]> {
        let p = self.take(3)?;
        Ok([p[0], p[1], p[2], 255])
    }

    fn palette(&mut self, n: usize) -> Result<Vec<[u8; 4]>> {
        (0..n).map(|_| self.cpixel()).collect()
    }

    /// Run length: sum of bytes until one is not 255, plus one.
    fn run_length(&mut self) -> Result<usize> {
        let mut len = 1;
        loop {
            let b = self.u8()?;
            len += b as usize;
            if b != 255 {
                return Ok(len);
            }
        }
    }
}

/// Decodes one ZRLE tile into `out` as RGBA.
fn decode_zrle_tile(data: &mut ByteReader, tile: Rect, out: &mut Vec<u8>) -> Result<()> {
    let pixels = tile.area();
    out.clear();
    let sub = data.u8()?;
    match sub {
        0 => {
            for p in data.take(pixels * 3)?.as_chunks::<3>().0 {
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        1 => {
            let c = data.cpixel()?;
            out.extend(std::iter::repeat_n(c, pixels).flatten());
        }
        2..=16 => {
            let palette = data.palette(sub as usize)?;
            let bits = match sub {
                2 => 1,
                3..=4 => 2,
                _ => 4,
            };
            let row_bytes = (tile.w as usize * bits).div_ceil(8);
            let packed = data.take(row_bytes * tile.h as usize)?;
            for row in packed.chunks_exact(row_bytes) {
                for x in 0..tile.w as usize {
                    let bit = x * bits;
                    let idx = (row[bit / 8] >> (8 - bits - bit % 8)) & ((1 << bits) - 1);
                    let c = palette
                        .get(idx as usize)
                        .ok_or_else(|| Error::Malformed("invalid palette index".into()))?;
                    out.extend_from_slice(c);
                }
            }
        }
        128 => {
            while out.len() < pixels * 4 {
                let c = data.cpixel()?;
                let run = data.run_length()?;
                push_run(out, c, run, pixels)?;
            }
        }
        130..=255 => {
            let palette = data.palette(sub as usize - 128)?;
            while out.len() < pixels * 4 {
                let idx = data.u8()?;
                let c = *palette
                    .get((idx & 0x7f) as usize)
                    .ok_or_else(|| Error::Malformed("invalid palette index".into()))?;
                let run = if idx & 0x80 != 0 {
                    data.run_length()?
                } else {
                    1
                };
                push_run(out, c, run, pixels)?;
            }
        }
        s => {
            return Err(Error::Malformed(format!("invalid ZRLE subencoding: {s}")));
        }
    }
    Ok(())
}

fn push_run(out: &mut Vec<u8>, c: [u8; 4], run: usize, pixels: usize) -> Result<()> {
    if out.len() / 4 + run > pixels {
        return Err(Error::Malformed("ZRLE run too long".into()));
    }
    out.extend(std::iter::repeat_n(c, run).flatten());
    Ok(())
}
