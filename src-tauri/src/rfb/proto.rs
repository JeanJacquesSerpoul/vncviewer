//! Handshake and client-to-server messages.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::security::{self, Security, SecurityOptions};
use super::{BoxStream, Error, Result};

pub mod encoding {
    pub const RAW: i32 = 0;
    pub const COPY_RECT: i32 = 1;
    pub const RRE: i32 = 2;
    pub const HEXTILE: i32 = 5;
    pub const TIGHT: i32 = 7;
    pub const ZRLE: i32 = 16;
    /// JPEG quality 0–9 (Tight): `QUALITY_0 + level`.
    pub const QUALITY_0: i32 = -32;
    /// zlib compression level 0–9 (Tight): `COMPRESSION_0 + level`.
    pub const COMPRESSION_0: i32 = -256;
    pub const DESKTOP_SIZE: i32 = -223;
    pub const LAST_RECT: i32 = -224;
    pub const CURSOR: i32 = -239;
    pub const EXTENDED_DESKTOP_SIZE: i32 = -308;
    pub const EXTENDED_CLIPBOARD: i32 = 0xC0A1E5CEu32 as i32;
}

/// Image quality / bandwidth trade-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    /// Lossless: ZRLE first, Tight without JPEG.
    #[default]
    Lossless,
    /// Tight with good JPEG quality.
    High,
    /// Tight with low JPEG quality and strong compression (slow links).
    Low,
}

/// Encodings announced to the server, in order of preference.
pub fn encodings(quality: Quality) -> Vec<i32> {
    use encoding::*;
    let mut v = vec![COPY_RECT];
    match quality {
        Quality::Lossless => v.extend([ZRLE, TIGHT]),
        Quality::High | Quality::Low => v.extend([TIGHT, ZRLE]),
    }
    v.extend([HEXTILE, RRE, RAW]);
    match quality {
        // No quality level: Tight stays lossless.
        Quality::Lossless => v.push(COMPRESSION_0 + 2),
        Quality::High => v.extend([QUALITY_0 + 8, COMPRESSION_0 + 2]),
        Quality::Low => v.extend([QUALITY_0 + 3, COMPRESSION_0 + 9]),
    }
    v.extend([
        CURSOR,
        EXTENDED_DESKTOP_SIZE,
        DESKTOP_SIZE,
        LAST_RECT,
        EXTENDED_CLIPBOARD,
    ]);
    v
}

const MAX_STRING: u32 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_colour: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// The format the client always requests: 32 bpp little-endian true colour,
    /// so every pixel arrives as the bytes R, G, B, X.
    pub const RGBX: Self = Self {
        bits_per_pixel: 32,
        depth: 24,
        big_endian: false,
        true_colour: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 0,
        green_shift: 8,
        blue_shift: 16,
    };

    pub fn to_bytes(self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = self.bits_per_pixel;
        b[1] = self.depth;
        b[2] = self.big_endian as u8;
        b[3] = self.true_colour as u8;
        b[4..6].copy_from_slice(&self.red_max.to_be_bytes());
        b[6..8].copy_from_slice(&self.green_max.to_be_bytes());
        b[8..10].copy_from_slice(&self.blue_max.to_be_bytes());
        b[10] = self.red_shift;
        b[11] = self.green_shift;
        b[12] = self.blue_shift;
        b
    }

    pub fn from_bytes(b: &[u8; 16]) -> Self {
        Self {
            bits_per_pixel: b[0],
            depth: b[1],
            big_endian: b[2] != 0,
            true_colour: b[3] != 0,
            red_max: u16::from_be_bytes([b[4], b[5]]),
            green_max: u16::from_be_bytes([b[6], b[7]]),
            blue_max: u16::from_be_bytes([b[8], b[9]]),
            red_shift: b[10],
            green_shift: b[11],
            blue_shift: b[12],
        }
    }
}

#[derive(Clone, Debug)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub pixel_format: PixelFormat,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Version {
    V3_3,
    V3_7,
    V3_8,
}

/// Runs the RFB handshake up to and including ServerInit. Returns the stream
/// to continue on (it may now be wrapped in TLS) and the security in effect.
pub async fn handshake(
    mut stream: BoxStream,
    opts: &SecurityOptions,
    shared: bool,
) -> Result<(BoxStream, ServerInit, Security)> {
    let version = negotiate_version(&mut stream).await?;
    let (mut stream, security) = security::negotiate(stream, version, opts).await?;

    stream.write_u8(shared as u8).await?;
    stream.flush().await?;

    let width = stream.read_u16().await?;
    let height = stream.read_u16().await?;
    let mut pf = [0u8; 16];
    stream.read_exact(&mut pf).await?;
    let name = read_string(&mut stream).await?;
    let init = ServerInit {
        width,
        height,
        pixel_format: PixelFormat::from_bytes(&pf),
        name,
    };
    Ok((stream, init, security))
}

async fn negotiate_version<S>(stream: &mut S) -> Result<Version>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut banner = [0u8; 12];
    stream.read_exact(&mut banner).await?;
    let text = std::str::from_utf8(&banner).unwrap_or_default();
    let parsed = text
        .strip_prefix("RFB ")
        .and_then(|v| v.strip_suffix('\n'))
        .and_then(|v| v.split_once('.'))
        .and_then(|(maj, min)| Some((maj.parse::<u32>().ok()?, min.parse::<u32>().ok()?)));
    let version = match parsed {
        Some((3, minor)) if minor >= 8 => Version::V3_8,
        Some((3, 7)) => Version::V3_7,
        Some((3, _)) => Version::V3_3,
        _ => {
            return Err(Error::NotVnc(text.trim_end().into()));
        }
    };
    let reply: &[u8] = match version {
        Version::V3_3 => b"RFB 003.003\n",
        Version::V3_7 => b"RFB 003.007\n",
        Version::V3_8 => b"RFB 003.008\n",
    };
    stream.write_all(reply).await?;
    stream.flush().await?;
    Ok(version)
}

pub(crate) async fn read_string<R: AsyncRead + Unpin>(r: &mut R) -> Result<String> {
    let len = r.read_u32().await?;
    if len > MAX_STRING {
        return Err(Error::Malformed(format!("string too long ({len} bytes)")));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(String::from_utf8(buf).unwrap_or_else(|e| latin1_decode(e.as_bytes())))
}

/// RFB clipboard text is ISO 8859-1.
pub fn latin1_decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

pub fn latin1_encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(c as u32).unwrap_or(b'?'))
        .collect()
}

// ---- Client messages -------------------------------------------------------

pub fn set_pixel_format(pf: &PixelFormat) -> Vec<u8> {
    let mut m = vec![0, 0, 0, 0];
    m.extend_from_slice(&pf.to_bytes());
    m
}

pub fn set_encodings(encodings: &[i32]) -> Vec<u8> {
    let mut m = vec![2, 0];
    m.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for e in encodings {
        m.extend_from_slice(&e.to_be_bytes());
    }
    m
}

pub fn update_request(incremental: bool, x: u16, y: u16, w: u16, h: u16) -> Vec<u8> {
    let mut m = vec![3, incremental as u8];
    for v in [x, y, w, h] {
        m.extend_from_slice(&v.to_be_bytes());
    }
    m
}

pub fn key_event(down: bool, keysym: u32) -> Vec<u8> {
    let mut m = vec![4, down as u8, 0, 0];
    m.extend_from_slice(&keysym.to_be_bytes());
    m
}

pub fn pointer_event(mask: u8, x: u16, y: u16) -> Vec<u8> {
    let mut m = vec![5, mask];
    m.extend_from_slice(&x.to_be_bytes());
    m.extend_from_slice(&y.to_be_bytes());
    m
}

/// Asks the server to resize its desktop (ExtendedDesktopSize), as one screen.
pub fn set_desktop_size(w: u16, h: u16, screen_id: u32) -> Vec<u8> {
    let mut m = vec![251, 0];
    m.extend_from_slice(&w.to_be_bytes());
    m.extend_from_slice(&h.to_be_bytes());
    m.extend_from_slice(&[1, 0]);
    m.extend_from_slice(&screen_id.to_be_bytes());
    for v in [0, 0, w, h] {
        m.extend_from_slice(&v.to_be_bytes());
    }
    m.extend_from_slice(&0u32.to_be_bytes());
    m
}

pub fn client_cut_text(text: &str) -> Vec<u8> {
    let bytes = latin1_encode(text);
    let mut m = vec![6, 0, 0, 0];
    m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    m.extend_from_slice(&bytes);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rfb::security::VNC_AUTH;

    fn opts(password: Option<&str>) -> SecurityOptions {
        SecurityOptions {
            password: password.map(Into::into),
            allow_vencrypt: true,
            ..Default::default()
        }
    }

    #[test]
    fn pixel_format_roundtrip() {
        assert_eq!(
            PixelFormat::from_bytes(&PixelFormat::RGBX.to_bytes()),
            PixelFormat::RGBX
        );
    }

    #[test]
    fn latin1_replaces_unrepresentable_chars() {
        assert_eq!(latin1_encode("é€"), vec![0xE9, b'?']);
        assert_eq!(latin1_decode(&[0x63, 0xE9]), "cé");
    }

    #[tokio::test]
    async fn rfb33_without_auth() {
        let (client, mut server) = tokio::io::duplex(1024);
        let srv = tokio::spawn(async move {
            server.write_all(b"RFB 003.003\n").await.unwrap();
            let mut v = [0u8; 12];
            server.read_exact(&mut v).await.unwrap();
            assert_eq!(&v, b"RFB 003.003\n");
            server.write_u32(1).await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), 0); // not shared
            server.write_all(&[0, 10, 0, 20]).await.unwrap();
            server
                .write_all(&PixelFormat::RGBX.to_bytes())
                .await
                .unwrap();
            server.write_u32(3).await.unwrap();
            server.write_all(b"old").await.unwrap();
        });
        let (_, init, security) = handshake(Box::new(client), &opts(None), false)
            .await
            .unwrap();
        assert_eq!(security, Security::None);
        srv.await.unwrap();
        assert_eq!(
            (init.width, init.height, init.name.as_str()),
            (10, 20, "old")
        );
    }

    #[tokio::test]
    async fn auth_failure_reports_reason() {
        let (client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut v = [0u8; 12];
            server.read_exact(&mut v).await.unwrap();
            server.write_all(&[1, VNC_AUTH as u8]).await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), VNC_AUTH as u8);
            server.write_all(&[7u8; 16]).await.unwrap();
            let mut resp = [0u8; 16];
            server.read_exact(&mut resp).await.unwrap();
            server.write_u32(1).await.unwrap();
            server.write_u32(5).await.unwrap();
            server.write_all(b"nope!").await.unwrap();
        });
        let err = handshake(Box::new(client), &opts(Some("secret")), true)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(err, Error::AuthFailed(Some(ref r)) if r == "nope!"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn missing_password_is_reported() {
        let (client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            server.write_all(b"RFB 003.008\n").await.unwrap();
            let mut v = [0u8; 12];
            server.read_exact(&mut v).await.unwrap();
            server.write_all(&[1, VNC_AUTH as u8]).await.unwrap();
            let _ = server.read_u8().await;
        });
        let err = handshake(Box::new(client), &opts(None), true)
            .await
            .err()
            .unwrap();
        assert!(matches!(err, Error::PasswordRequired), "{err:?}");
    }
}
