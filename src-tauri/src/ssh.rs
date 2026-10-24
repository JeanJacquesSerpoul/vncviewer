//! SSH tunnel: the VNC connection goes through a `direct-tcpip` channel.
//!
//! The host key is checked against `~/.ssh/known_hosts`; an unknown key is
//! refused with its fingerprint, and recorded once the user trusts it.
//! Authentication tries the SSH agent, then unencrypted keys in `~/.ssh`,
//! then the password if one was given.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use russh::ChannelStream;
use russh::client::{self, Handle, Msg};
use russh::keys::PublicKeyOrCertificate;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, known_hosts};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::rfb::{BoxStream, Error, Result};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshParams {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub password: Option<String>,
    /// Fingerprint of an unknown host key the user accepted.
    #[serde(default)]
    pub trusted_key: Option<String>,
}

fn default_port() -> u16 {
    22
}

const KEY_FILES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

struct Client {
    host: String,
    port: u16,
    trusted: Option<String>,
    /// Why the host key was refused.
    refused: Arc<Mutex<Option<Error>>>,
}

fn key_fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            *self.refused.lock().unwrap() =
                Some(Error::Ssh("host certificates are not supported".into()));
            return Ok(false);
        };
        let fp = key_fingerprint(key);
        let refusal = match known_hosts::check_known_hosts(&self.host, self.port, key) {
            Ok(true) => return Ok(true),
            Ok(false) if self.trusted.as_deref() == Some(fp.as_str()) => {
                if let Err(e) = known_hosts::learn_known_hosts(&self.host, self.port, key) {
                    log::warn!("cannot record the SSH host key: {e}");
                }
                return Ok(true);
            }
            Ok(false) => Error::SshHostKeyUnknown(fp),
            Err(russh::keys::Error::KeyChanged { .. }) => Error::SshHostKeyChanged(fp),
            Err(e) => Error::Ssh(format!("known_hosts: {e}")),
        };
        *self.refused.lock().unwrap() = Some(refusal);
        Ok(false)
    }
}

/// Connects to the SSH server and opens a channel to `host:port` seen from it.
pub async fn tunnel(p: &SshParams, host: &str, port: u16) -> Result<BoxStream> {
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        nodelay: true,
        ..Default::default()
    });
    let refused = Arc::new(Mutex::new(None));
    let handler = Client {
        host: p.host.clone(),
        port: p.port,
        trusted: p.trusted_key.clone(),
        refused: refused.clone(),
    };
    let mut session = client::connect(config, (p.host.as_str(), p.port), handler)
        .await
        .map_err(|e| match refused.lock().unwrap().take() {
            Some(refusal) => refusal,
            None => match e {
                russh::Error::IO(io) => Error::Io(io),
                e => Error::Ssh(e.to_string()),
            },
        })?;

    if !authenticate(&mut session, p).await? {
        return Err(Error::SshAuthFailed);
    }
    let channel = session
        .channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0)
        .await
        .map_err(|e| Error::Ssh(format!("cannot reach {host}:{port}: {e}")))?;
    Ok(Box::new(Tunnel {
        stream: channel.into_stream(),
        _session: session,
    }))
}

async fn authenticate(session: &mut Handle<Client>, p: &SshParams) -> Result<bool> {
    let ssh_err = |e: russh::Error| Error::Ssh(e.to_string());
    let rsa_hash = session
        .best_supported_rsa_hash()
        .await
        .map_err(ssh_err)?
        .flatten();

    if let Ok(mut agent) = agent().await
        && let Ok(identities) = agent.request_identities().await
    {
        for id in identities {
            let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = id else {
                continue;
            };
            match session
                .authenticate_publickey_with(&p.user, key, rsa_hash, &mut agent)
                .await
            {
                Ok(r) if r.success() => return Ok(true),
                Ok(_) => {}
                Err(e) => log::debug!("SSH agent key refused: {e}"),
            }
        }
    }

    for path in key_paths() {
        // Keys protected by a passphrase are skipped.
        let Ok(key) = russh::keys::load_secret_key(&path, None) else {
            continue;
        };
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash);
        let r = session
            .authenticate_publickey(&p.user, key)
            .await
            .map_err(ssh_err)?;
        if r.success() {
            return Ok(true);
        }
    }

    if let Some(password) = &p.password {
        let r = session
            .authenticate_password(&p.user, password)
            .await
            .map_err(ssh_err)?;
        return Ok(r.success());
    }
    Ok(false)
}

#[cfg(unix)]
async fn agent() -> std::result::Result<
    russh::keys::agent::client::AgentClient<tokio::net::UnixStream>,
    russh::keys::Error,
> {
    russh::keys::agent::client::AgentClient::connect_env().await
}

#[cfg(windows)]
async fn agent() -> std::result::Result<
    russh::keys::agent::client::AgentClient<tokio::net::windows::named_pipe::NamedPipeClient>,
    russh::keys::Error,
> {
    russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await
}

fn key_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::home_dir() else {
        return Vec::new();
    };
    KEY_FILES
        .iter()
        .map(|f| home.join(".ssh").join(f))
        .filter(|p| p.exists())
        .collect()
}

/// The channel, keeping the SSH session alive as long as it is used.
struct Tunnel {
    stream: ChannelStream<Msg>,
    _session: Handle<Client>,
}

impl AsyncRead for Tunnel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for Tunnel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
