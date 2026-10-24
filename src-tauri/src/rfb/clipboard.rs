//! Extended clipboard pseudo-encoding (UTF-8 text instead of ISO 8859-1).
//!
//! An extended message is a cut text message whose length, read as a signed
//! integer, is negative: its payload starts with a `u32` of flags (one action
//! in the high byte, formats in the low 16 bits). Provided data is a separate
//! zlib stream holding, for each format, a `u32` size followed by the data.

use flate2::{Compress, Compression, Decompress, FlushCompress};

use super::decode::inflate;
use super::{Error, Result};

pub const TEXT: u32 = 1;
pub const CAPS: u32 = 1 << 24;
pub const REQUEST: u32 = 1 << 25;
pub const PEEK: u32 = 1 << 26;
pub const NOTIFY: u32 = 1 << 27;
pub const PROVIDE: u32 = 1 << 28;
/// Largest clipboard text accepted or announced.
pub const MAX_TEXT: u32 = 16 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The server supports the extension; `text` if it handles UTF-8 text.
    Caps { text: bool },
    /// The server's clipboard now holds `formats` (we may request them).
    Notify { text: bool },
    /// The server asks for our clipboard.
    Request { text: bool },
    /// The server asks which formats we hold.
    Peek,
    /// Clipboard text from the server (`None` if no text was provided).
    Provide(Option<String>),
}

/// Parses the payload of an extended cut text message.
pub fn parse(payload: &[u8]) -> Result<Message> {
    let flags = payload
        .first_chunk::<4>()
        .map(|b| u32::from_be_bytes(*b))
        .ok_or_else(|| Error::Malformed("short extended clipboard message".into()))?;
    let text = flags & TEXT != 0;
    let data = &payload[4..];
    Ok(if flags & CAPS != 0 {
        Message::Caps { text }
    } else if flags & PROVIDE != 0 {
        Message::Provide(if text { provided_text(data)? } else { None })
    } else if flags & NOTIFY != 0 {
        Message::Notify { text }
    } else if flags & REQUEST != 0 {
        Message::Request { text }
    } else if flags & PEEK != 0 {
        Message::Peek
    } else {
        return Err(Error::Malformed(format!(
            "unknown extended clipboard action {flags:#x}"
        )));
    })
}

fn provided_text(compressed: &[u8]) -> Result<Option<String>> {
    let mut data = Vec::new();
    inflate(&mut Decompress::new(true), compressed, &mut data)?;
    let Some(size) = data.first_chunk::<4>().map(|b| u32::from_be_bytes(*b)) else {
        return Ok(None);
    };
    if size > MAX_TEXT || data.len() < 4 + size as usize {
        return Err(Error::Malformed("invalid extended clipboard text".into()));
    }
    let bytes = &data[4..4 + size as usize];
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    Ok(Some(String::from_utf8_lossy(bytes).replace("\r\n", "\n")))
}

fn message(flags: u32, payload: &[u8]) -> Vec<u8> {
    let len = -((4 + payload.len()) as i32);
    let mut m = vec![6, 0, 0, 0];
    m.extend_from_slice(&len.to_be_bytes());
    m.extend_from_slice(&flags.to_be_bytes());
    m.extend_from_slice(payload);
    m
}

/// Our capabilities: UTF-8 text, and the actions we understand.
pub fn caps() -> Vec<u8> {
    message(
        CAPS | TEXT | REQUEST | PEEK | NOTIFY | PROVIDE,
        &MAX_TEXT.to_be_bytes(),
    )
}

pub fn notify(has_text: bool) -> Vec<u8> {
    message(NOTIFY | if has_text { TEXT } else { 0 }, &[])
}

pub fn request_text() -> Vec<u8> {
    message(REQUEST | TEXT, &[])
}

/// Provides `text` (UTF-8, CRLF line endings, NUL-terminated as the spec asks).
pub fn provide_text(text: &str) -> Vec<u8> {
    let mut body = text
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .into_bytes();
    body.push(0);
    let mut plain = (body.len() as u32).to_be_bytes().to_vec();
    plain.extend_from_slice(&body);
    let mut z = Compress::new(Compression::default(), true);
    let mut out = Vec::with_capacity(plain.len() + 64);
    loop {
        let consumed = z.total_in() as usize;
        z.compress_vec(&plain[consumed..], &mut out, FlushCompress::Finish)
            .expect("in-memory zlib");
        if z.total_in() as usize == plain.len() && out.len() < out.capacity() {
            break;
        }
        out.reserve(out.capacity());
    }
    message(PROVIDE | TEXT, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splits a client message into (flags, payload after the flags).
    fn unwrap(m: &[u8]) -> (u32, &[u8]) {
        assert_eq!(m[0], 6);
        let len = i32::from_be_bytes(m[4..8].try_into().unwrap());
        assert_eq!(-len as usize, m.len() - 8);
        (u32::from_be_bytes(m[8..12].try_into().unwrap()), &m[12..])
    }

    #[test]
    fn provide_round_trip() {
        let m = provide_text("héllo\nwörld 🎉");
        let (flags, payload) = unwrap(&m);
        assert_eq!(flags, PROVIDE | TEXT);
        let mut all = flags.to_be_bytes().to_vec();
        all.extend_from_slice(payload);
        assert_eq!(
            parse(&all).unwrap(),
            Message::Provide(Some("héllo\nwörld 🎉".into()))
        );
    }

    #[test]
    fn parses_actions() {
        let m = |flags: u32| flags.to_be_bytes().to_vec();
        assert_eq!(
            parse(&m(CAPS | TEXT | 4)).unwrap(),
            Message::Caps { text: true }
        );
        assert_eq!(
            parse(&m(NOTIFY | TEXT)).unwrap(),
            Message::Notify { text: true }
        );
        assert_eq!(
            parse(&m(REQUEST | TEXT)).unwrap(),
            Message::Request { text: true }
        );
        assert_eq!(parse(&m(PEEK)).unwrap(), Message::Peek);
        assert!(parse(&[0, 0]).is_err());
        let caps = caps();
        let (flags, payload) = unwrap(&caps);
        assert_eq!(flags & (CAPS | TEXT), CAPS | TEXT);
        assert_eq!(payload, MAX_TEXT.to_be_bytes());
    }
}
