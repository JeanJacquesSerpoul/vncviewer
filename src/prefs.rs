//! Last connection settings, remembered in localStorage (never the password).

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

const KEY: &str = "vncviewer.connection";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub host: String,
    pub port: u16,
    pub shared: bool,
    pub fit: bool,
    /// `None` until chosen: follow the system language.
    pub lang: Option<Lang>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5900,
            shared: true,
            fit: true,
            lang: None,
        }
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
