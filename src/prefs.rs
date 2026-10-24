//! Settings and connection history, remembered in localStorage (never a
//! password: those go to the system keychain, see [`crate::secrets`]).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

const KEY: &str = "vncviewer.connection";
/// Non-favourite servers kept in the history.
const MAX_RECENT: usize = 10;

/// Display scale: `0` fits the window, otherwise a percentage.
pub type Zoom = u16;
pub const FIT: Zoom = 0;
pub const ZOOMS: [Zoom; 7] = [FIT, 50, 75, 100, 125, 150, 200];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    #[default]
    Lossless,
    High,
    Low,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Lossless, Quality::High, Quality::Low];

    pub fn key(self) -> &'static str {
        match self {
            Quality::Lossless => "lossless",
            Quality::High => "high",
            Quality::Low => "low",
        }
    }

    pub fn from_key(k: &str) -> Quality {
        Quality::ALL
            .into_iter()
            .find(|q| q.key() == k)
            .unwrap_or_default()
    }
}

/// A server in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Server {
    /// The server field as typed (`host`, `host:1`…).
    pub address: String,
    /// SSH tunnel, as typed (`user@host[:port]`), empty for none.
    pub ssh: String,
    /// User name, for servers that ask for one (macOS, VeNCrypt Plain).
    pub username: String,
    /// Desktop name reported by the server.
    pub name: String,
    pub favorite: bool,
    /// The password is saved in the keychain.
    pub remember: bool,
    /// Last connection (ms since the epoch).
    pub last_used: f64,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            address: String::new(),
            ssh: String::new(),
            username: String::new(),
            name: String::new(),
            favorite: false,
            remember: false,
            last_used: 0.0,
        }
    }
}

impl Server {
    /// Two entries are the same server if they connect the same way.
    pub fn same_target(&self, other: &Server) -> bool {
        self.address
            .trim()
            .eq_ignore_ascii_case(other.address.trim())
            && self.ssh.trim() == other.ssh.trim()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub host: String,
    /// Port used when the server field has none.
    pub port: u16,
    pub ssh: String,
    pub username: String,
    pub shared: bool,
    /// Legacy setting (before zoom levels), read once.
    #[serde(skip_serializing)]
    pub fit: Option<bool>,
    pub zoom: Option<Zoom>,
    pub smooth: bool,
    pub quality: Quality,
    /// Resize the remote desktop to the window, when the server allows it.
    pub remote_resize: bool,
    pub auto_reconnect: bool,
    /// `None` until chosen: follow the system language.
    pub lang: Option<Lang>,
    pub servers: Vec<Server>,
    /// Trusted certificate fingerprints, by `host:port`.
    pub trusted_certs: HashMap<String, String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5900,
            ssh: String::new(),
            username: String::new(),
            shared: true,
            fit: None,
            zoom: None,
            smooth: true,
            quality: Quality::Lossless,
            remote_resize: false,
            auto_reconnect: true,
            lang: None,
            servers: Vec::new(),
            trusted_certs: HashMap::new(),
        }
    }
}

impl Prefs {
    pub fn zoom(&self) -> Zoom {
        self.zoom.unwrap_or(match self.fit {
            Some(false) => 100,
            _ => FIT,
        })
    }

    /// Records a successful connection at the top of the history.
    pub fn remember_server(&mut self, mut server: Server) {
        if let Some(i) = self.servers.iter().position(|s| s.same_target(&server)) {
            let old = self.servers.remove(i);
            server.favorite |= old.favorite;
        }
        self.servers.insert(0, server);
        let mut recent = 0;
        self.servers.retain(|s| {
            recent += !s.favorite as usize;
            s.favorite || recent <= MAX_RECENT
        });
    }

    /// Favourites first, then by last use.
    pub fn sorted_servers(&self) -> Vec<Server> {
        let mut v = self.servers.clone();
        v.sort_by(|a, b| {
            b.favorite
                .cmp(&a.favorite)
                .then(b.last_used.total_cmp(&a.last_used))
        });
        v
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn load() -> Prefs {
    storage()
        .and_then(|s| s.get_item(KEY).ok()?)
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

pub fn save(p: &Prefs) {
    if let (Some(s), Ok(v)) = (storage(), serde_json::to_string(p)) {
        let _ = s.set_item(KEY, &v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(address: &str, favorite: bool, last_used: f64) -> Server {
        Server {
            address: address.into(),
            favorite,
            last_used,
            ..Default::default()
        }
    }

    #[test]
    fn old_settings_are_read() {
        let p: Prefs =
            serde_json::from_str(r#"{"host":"pc","port":5901,"shared":false,"fit":false}"#)
                .unwrap();
        assert_eq!((p.host.as_str(), p.port, p.shared), ("pc", 5901, false));
        assert_eq!(p.zoom(), 100);
        assert_eq!(Prefs::default().zoom(), FIT);
        assert!(p.servers.is_empty());
    }

    #[test]
    fn history_keeps_favourites_and_recent_servers() {
        let mut p = Prefs::default();
        p.remember_server(server("fav", true, 0.0));
        for i in 0..15 {
            p.remember_server(server(&format!("pc{i}"), false, i as f64));
        }
        assert_eq!(p.servers.len(), 1 + MAX_RECENT);
        assert!(p.servers.iter().any(|s| s.address == "fav"));
        // Reconnecting moves it to the top and keeps it a favourite.
        p.remember_server(server("FAV", false, 99.0));
        let sorted = p.sorted_servers();
        assert_eq!(sorted[0].address, "FAV");
        assert!(sorted[0].favorite);
        assert_eq!(sorted[1].address, "pc14");
        assert_eq!(p.servers.len(), 1 + MAX_RECENT);
    }
}
