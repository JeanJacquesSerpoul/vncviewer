//! Security handshake: choice of the security type and authentication.
//!
//! Supported: None, VNC authentication, Apple Remote Desktop (ARD) and VeNCrypt
//! with an X.509 TLS session (X509None, X509Vnc, X509Plain). Anonymous TLS
//! (TLSNone, TLSVnc…) is not offered by rustls, so it is not supported.

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::proto::{Version, read_string};
use super::{BoxStream, Error, Result, ard, auth, tls};

pub const NONE: u32 = 1;
pub const VNC_AUTH: u32 = 2;
pub const VENCRYPT: u32 = 19;
pub const ARD: u32 = 30;

/// VeNCrypt subtypes.
pub mod vencrypt {
    pub const PLAIN: u32 = 256;
    pub const TLS_NONE: u32 = 257;
    pub const TLS_VNC: u32 = 258;
    pub const TLS_PLAIN: u32 = 259;
    pub const X509_NONE: u32 = 260;
    pub const X509_VNC: u32 = 261;
    pub const X509_PLAIN: u32 = 262;
}

/// What the client may use to authenticate.
#[derive(Debug, Clone, Default)]
pub struct SecurityOptions {
    pub username: Option<String>,
    pub password: Option<String>,
    /// Server name, checked against its TLS certificate.
    pub host: String,
    /// SHA-256 fingerprint of a certificate the user chose to trust.
    pub trusted_cert: Option<String>,
    /// Whether VeNCrypt may be chosen (disabled to fall back after a failure).
    pub allow_vencrypt: bool,
}

/// The security in effect once the handshake is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    None,
    Vnc,
    Ard,
    X509None,
    X509Vnc,
    X509Plain,
}

impl Security {
    /// Whether the whole session is encrypted (not only the credentials).
    pub fn encrypted(self) -> bool {
        matches!(
            self,
            Security::X509None | Security::X509Vnc | Security::X509Plain
        )
    }
}

/// Names of security types, for error messages.
pub fn describe(types: &[u32]) -> String {
    types
        .iter()
        .map(|&t| {
            let name = match t {
                NONE => "None",
                VNC_AUTH => "VNC",
                5 => "RA2",
                6 => "RA2ne",
                16 => "Tight",
                17 => "Ultra",
                18 => "TLS",
                VENCRYPT => "VeNCrypt",
                ARD => "Apple",
                113 => "MSLogon II",
                vencrypt::PLAIN => "Plain",
                vencrypt::TLS_NONE => "TLSNone",
                vencrypt::TLS_VNC => "TLSVnc",
                vencrypt::TLS_PLAIN => "TLSPlain",
                vencrypt::X509_NONE => "X509None",
                vencrypt::X509_VNC => "X509Vnc",
                vencrypt::X509_PLAIN => "X509Plain",
                _ => return t.to_string(),
            };
            format!("{t} ({name})")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Negotiates the security type and authenticates, up to and including the
/// SecurityResult. Returns the stream to use afterwards (TLS may wrap it).
pub async fn negotiate(
    mut s: BoxStream,
    version: Version,
    opts: &SecurityOptions,
) -> Result<(BoxStream, Security)> {
    let chosen = if version == Version::V3_3 {
        // The server imposes the security type.
        match s.read_u32().await? {
            0 => return Err(Error::Server(read_string(&mut s).await?)),
            NONE => NONE,
            VNC_AUTH if opts.password.is_none() => return Err(Error::PasswordRequired),
            VNC_AUTH => VNC_AUTH,
            t => return Err(Error::UnsupportedSecurity(vec![t])),
        }
    } else {
        let count = s.read_u8().await?;
        if count == 0 {
            return Err(Error::Server(read_string(&mut s).await?));
        }
        let mut types = vec![0u8; count as usize];
        s.read_exact(&mut types).await?;
        let types: Vec<u32> = types.into_iter().map(u32::from).collect();
        let chosen = choose(&types, opts)?;
        s.write_u8(chosen as u8).await?;
        chosen
    };

    let (mut s, security) = match chosen {
        NONE => (s, Security::None),
        VNC_AUTH => {
            vnc_auth(&mut s, opts).await?;
            (s, Security::Vnc)
        }
        ARD => {
            let (user, pass) = credentials(opts)?;
            ard::authenticate(&mut s, user, pass).await?;
            (s, Security::Ard)
        }
        _ => vencrypt(s, opts).await?,
    };

    // RFB 3.8 always sends a SecurityResult; older versions only after authentication.
    if (version == Version::V3_8 || security != Security::None) && s.read_u32().await? != 0 {
        let reason = if version == Version::V3_8 {
            Some(read_string(&mut s).await?).filter(|r| !r.is_empty())
        } else {
            None
        };
        return Err(Error::AuthFailed(reason));
    }
    Ok((s, security))
}

/// Picks a security type among those offered, encrypted ones first.
fn choose(types: &[u32], opts: &SecurityOptions) -> Result<u32> {
    let offers = |t| types.contains(&t);
    let has_password = opts.password.is_some();
    let has_credentials = has_password && opts.username.is_some();
    Ok(if opts.allow_vencrypt && offers(VENCRYPT) {
        VENCRYPT
    } else if offers(ARD) && has_credentials {
        ARD
    } else if offers(VNC_AUTH) && has_password {
        VNC_AUTH
    } else if offers(NONE) {
        NONE
    } else if offers(VNC_AUTH) {
        return Err(Error::PasswordRequired);
    } else if offers(ARD) {
        return Err(Error::CredentialsRequired);
    } else {
        return Err(Error::UnsupportedSecurity(types.to_vec()));
    })
}

fn credentials(opts: &SecurityOptions) -> Result<(&str, &str)> {
    match (&opts.username, &opts.password) {
        (Some(u), Some(p)) => Ok((u, p)),
        _ => Err(Error::CredentialsRequired),
    }
}

async fn vnc_auth<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    opts: &SecurityOptions,
) -> Result<()> {
    let password = opts.password.as_deref().ok_or(Error::PasswordRequired)?;
    let mut challenge = [0u8; 16];
    s.read_exact(&mut challenge).await?;
    s.write_all(&auth::challenge_response(password, &challenge))
        .await?;
    Ok(())
}

async fn vencrypt(mut s: BoxStream, opts: &SecurityOptions) -> Result<(BoxStream, Security)> {
    let (major, minor) = (s.read_u8().await?, s.read_u8().await?);
    if (major, minor) < (0, 2) {
        return Err(Error::Malformed(format!(
            "VeNCrypt version {major}.{minor} not supported"
        )));
    }
    s.write_all(&[0, 2]).await?;
    s.flush().await?;
    if s.read_u8().await? != 0 {
        return Err(Error::Server("VeNCrypt version refused".into()));
    }
    let count = s.read_u8().await?;
    let mut subtypes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        subtypes.push(s.read_u32().await?);
    }
    let sub = choose_vencrypt(&subtypes, opts)?;
    s.write_u32(sub).await?;
    s.flush().await?;
    if s.read_u8().await? != 1 {
        return Err(Error::Server("TLS session refused".into()));
    }

    let mut s: BoxStream =
        Box::new(tls::connect(s, &opts.host, opts.trusted_cert.as_deref()).await?);
    let security = match sub {
        vencrypt::X509_VNC => {
            vnc_auth(&mut s, opts).await?;
            Security::X509Vnc
        }
        vencrypt::X509_PLAIN => {
            let (user, pass) = credentials(opts)?;
            s.write_u32(user.len() as u32).await?;
            s.write_u32(pass.len() as u32).await?;
            s.write_all(user.as_bytes()).await?;
            s.write_all(pass.as_bytes()).await?;
            Security::X509Plain
        }
        _ => Security::X509None,
    };
    s.flush().await?;
    Ok((s, security))
}

fn choose_vencrypt(subtypes: &[u32], opts: &SecurityOptions) -> Result<u32> {
    use vencrypt::*;
    let offers = |t| subtypes.contains(&t);
    let has_password = opts.password.is_some();
    let has_credentials = has_password && opts.username.is_some();
    Ok(if offers(X509_VNC) && has_password {
        X509_VNC
    } else if offers(X509_PLAIN) && has_credentials {
        X509_PLAIN
    } else if offers(X509_NONE) {
        X509_NONE
    } else if offers(X509_VNC) {
        return Err(Error::PasswordRequired);
    } else if offers(X509_PLAIN) {
        return Err(Error::CredentialsRequired);
    } else {
        return Err(Error::VencryptUnusable(subtypes.to_vec()));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(user: Option<&str>, pass: Option<&str>) -> SecurityOptions {
        SecurityOptions {
            username: user.map(Into::into),
            password: pass.map(Into::into),
            allow_vencrypt: true,
            ..Default::default()
        }
    }

    #[test]
    fn prefers_encryption_then_credentials() {
        let all = [NONE, VNC_AUTH, VENCRYPT, ARD];
        assert_eq!(choose(&all, &opts(None, None)).unwrap(), VENCRYPT);
        let no_tls = SecurityOptions {
            allow_vencrypt: false,
            ..opts(Some("u"), Some("p"))
        };
        assert_eq!(choose(&all, &no_tls).unwrap(), ARD);
        assert_eq!(
            choose(&[NONE, VNC_AUTH], &opts(None, Some("p"))).unwrap(),
            VNC_AUTH
        );
        assert_eq!(choose(&[NONE, VNC_AUTH], &opts(None, None)).unwrap(), NONE);
        assert!(matches!(
            choose(&[ARD, VNC_AUTH], &opts(None, None)),
            Err(Error::PasswordRequired)
        ));
        assert!(matches!(
            choose(&[ARD], &opts(None, Some("p"))),
            Err(Error::CredentialsRequired)
        ));
    }

    #[test]
    fn anonymous_tls_is_unusable() {
        use vencrypt::*;
        assert!(matches!(
            choose_vencrypt(&[TLS_VNC, TLS_NONE], &opts(None, Some("p"))),
            Err(Error::VencryptUnusable(_))
        ));
        assert_eq!(
            choose_vencrypt(&[TLS_VNC, X509_NONE, X509_VNC], &opts(None, Some("p"))).unwrap(),
            X509_VNC
        );
        assert!(matches!(
            choose_vencrypt(&[X509_PLAIN], &opts(None, Some("p"))),
            Err(Error::CredentialsRequired)
        ));
    }
}
