//! Binary messages sent by the backend session (see `src-tauri/src/session.rs`).

use serde::Deserialize;

pub const MSG_EVENT: u8 = 0;
pub const MSG_RECTS: u8 = 1;
pub const MSG_CURSOR: u8 = 2;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Resize {
        width: u32,
        height: u32,
        name: String,
    },
    Bell,
    Disconnected {
        reason: Option<ErrorInfo>,
    },
}

/// Language-neutral error sent by the backend (`rfb::ErrorInfo`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// Random access to little-endian message bytes, so that the parsing works on
/// a JS `DataView` (no copy into wasm memory) as well as on a Rust slice.
pub trait Bytes {
    fn len(&self) -> usize;
    fn u8_at(&self, i: usize) -> u8;
    fn u16_at(&self, i: usize) -> u16;
    fn u32_at(&self, i: usize) -> u32;
}

impl Bytes for [u8] {
    fn len(&self) -> usize {
        <[u8]>::len(self)
    }
    fn u8_at(&self, i: usize) -> u8 {
        self[i]
    }
    fn u16_at(&self, i: usize) -> u16 {
        u16::from_le_bytes([self[i], self[i + 1]])
    }
    fn u32_at(&self, i: usize) -> u32 {
        u32::from_le_bytes(self[i..i + 4].try_into().unwrap())
    }
}

/// A block of RGBA pixels inside a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelBlock {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Byte offset of the pixel data within the message.
    pub offset: usize,
}

impl PixelBlock {
    pub fn byte_len(&self) -> usize {
        self.w as usize * self.h as usize * 4
    }
}

fn block_at<B: Bytes + ?Sized>(b: &B, at: usize) -> Result<PixelBlock, String> {
    if at + 8 > b.len() {
        return Err("en-tête de rectangle tronqué".into());
    }
    let block = PixelBlock {
        x: b.u16_at(at) as u32,
        y: b.u16_at(at + 2) as u32,
        w: b.u16_at(at + 4) as u32,
        h: b.u16_at(at + 6) as u32,
        offset: at + 8,
    };
    if block.offset + block.byte_len() > b.len() {
        return Err("pixels tronqués".into());
    }
    Ok(block)
}

/// Rectangles of a `MSG_RECTS` message.
pub fn rects<B: Bytes + ?Sized>(b: &B) -> Result<Vec<PixelBlock>, String> {
    if b.len() < 5 || b.u8_at(0) != MSG_RECTS {
        return Err("message de rectangles invalide".into());
    }
    let count = b.u32_at(1) as usize;
    let mut out = Vec::with_capacity(count.min(1024));
    let mut at = 5;
    for _ in 0..count {
        let block = block_at(b, at)?;
        at = block.offset + block.byte_len();
        out.push(block);
    }
    Ok(out)
}

/// Cursor of a `MSG_CURSOR` message: the block's x/y are the hotspot.
pub fn cursor<B: Bytes + ?Sized>(b: &B) -> Result<PixelBlock, String> {
    if b.len() < 1 || b.u8_at(0) != MSG_CURSOR {
        return Err("message de curseur invalide".into());
    }
    block_at(b, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rects() {
        let mut m = vec![MSG_RECTS, 2, 0, 0, 0];
        m.extend_from_slice(&[1, 0, 2, 0, 1, 0, 1, 0]);
        m.extend_from_slice(&[9; 4]);
        m.extend_from_slice(&[0, 1, 0, 0, 2, 0, 1, 0]);
        m.extend_from_slice(&[7; 8]);
        let r = rects(&m[..]).unwrap();
        assert_eq!(
            r[0],
            PixelBlock {
                x: 1,
                y: 2,
                w: 1,
                h: 1,
                offset: 13
            }
        );
        assert_eq!(
            r[1],
            PixelBlock {
                x: 256,
                y: 0,
                w: 2,
                h: 1,
                offset: 25
            }
        );
    }

    #[test]
    fn rejects_truncated_pixels() {
        let mut m = vec![MSG_RECTS, 1, 0, 0, 0];
        m.extend_from_slice(&[0, 0, 0, 0, 2, 0, 2, 0]);
        m.extend_from_slice(&[0; 15]);
        assert!(rects(&m[..]).is_err());
    }

    #[test]
    fn parses_events() {
        let ev: Event = serde_json::from_str(r#"{"type":"disconnected","reason":null}"#).unwrap();
        assert_eq!(ev, Event::Disconnected { reason: None });
        let ev: Event =
            serde_json::from_str(r#"{"type":"disconnected","reason":{"code":"timeout"}}"#).unwrap();
        assert_eq!(
            ev,
            Event::Disconnected {
                reason: Some(ErrorInfo {
                    code: "timeout".into(),
                    detail: None
                })
            }
        );
    }
}
