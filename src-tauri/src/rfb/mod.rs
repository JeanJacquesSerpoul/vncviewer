//! Minimal RFB (VNC) client protocol implementation (RFC 6143).

pub mod auth;
pub mod decode;
pub mod framebuffer;
pub mod proto;

pub use decode::{CursorShape, Decoder, ServerMessage};
pub use framebuffer::{Framebuffer, Rect};
pub use proto::{ServerInit, handshake};

/// Connection errors. `Display` (English) is for logs; the UI receives an
/// [`ErrorInfo`] and translates it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a VNC server ({0:?})")]
    NotVnc(String),
    /// Reason given by the server for refusing the connection.
    #[error("server refused the connection: {0}")]
    Server(String),
    #[error("authentication failed ({0:?})")]
    AuthFailed(Option<String>),
    #[error("password required")]
    PasswordRequired,
    #[error("no supported security type (offered: {0:?})")]
    UnsupportedSecurity(Vec<u32>),
    #[error("unsupported encoding {0}")]
    UnsupportedEncoding(i32),
    #[error("screen too large ({0}×{1})")]
    ScreenTooLarge(u16, u16),
    #[error("unknown server message type {0}")]
    UnknownMessage(u8),
    /// Invalid data from the server; the detail is technical.
    #[error("invalid data from the server: {0}")]
    Malformed(String),
    #[error("connection lost")]
    ConnectionLost,
}

/// Language-neutral description of an [`Error`], sent to the UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ErrorInfo {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Error {
    pub fn info(&self) -> ErrorInfo {
        use std::io::ErrorKind;
        let (code, detail) = match self {
            Error::Io(e) => match e.kind() {
                ErrorKind::UnexpectedEof => ("serverClosed", None),
                ErrorKind::TimedOut => ("timeout", None),
                ErrorKind::ConnectionRefused => ("refused", None),
                _ => ("network", Some(e.to_string())),
            },
            Error::NotVnc(banner) => ("notVnc", Some(banner.clone())),
            Error::Server(reason) => ("serverRefused", Some(reason.clone())),
            Error::AuthFailed(reason) => ("authFailed", reason.clone()),
            Error::PasswordRequired => ("passwordRequired", None),
            Error::UnsupportedSecurity(types) => (
                "unsupportedSecurity",
                Some(
                    types
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ),
            Error::UnsupportedEncoding(e) => ("unsupportedEncoding", Some(e.to_string())),
            Error::ScreenTooLarge(w, h) => ("screenTooLarge", Some(format!("{w}×{h}"))),
            Error::UnknownMessage(t) => ("unknownMessage", Some(t.to_string())),
            Error::Malformed(d) => ("malformed", Some(d.clone())),
            Error::ConnectionLost => ("connectionLost", None),
        };
        ErrorInfo { code, detail }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_language_neutral_codes() {
        let eof = Error::Io(std::io::ErrorKind::UnexpectedEof.into());
        assert_eq!(eof.info(), ErrorInfo { code: "serverClosed", detail: None });
        let e = Error::UnsupportedSecurity(vec![16, 19]);
        assert_eq!(e.info().detail.as_deref(), Some("16, 19"));
        let json = serde_json::to_string(&Error::AuthFailed(None).info()).unwrap();
        assert_eq!(json, r#"{"code":"authFailed"}"#);
    }
}
