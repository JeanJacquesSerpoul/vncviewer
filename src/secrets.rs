//! Passwords saved in the system keychain (through the backend).

use serde::Serialize;

use crate::tauri;

#[derive(Serialize)]
struct Key<'a> {
    key: &'a str,
}

#[derive(Serialize)]
struct Entry<'a> {
    key: &'a str,
    secret: &'a str,
}

/// Keychain entry of a VNC server's password.
pub fn vnc_key(address: &str, username: &str, ssh: &str) -> String {
    let mut k = format!("vnc {}", address.trim());
    if !username.trim().is_empty() {
        k = format!("vnc {}@{}", username.trim(), address.trim());
    }
    if !ssh.trim().is_empty() {
        k.push_str(" via ");
        k.push_str(ssh.trim());
    }
    k
}

/// Keychain entry of an SSH gateway's password.
pub fn ssh_key(target: &str) -> String {
    format!("ssh {}", target.trim())
}

pub async fn get(key: &str) -> Option<String> {
    match tauri::invoke("secret_get", &Key { key }).await {
        Ok(v) => v.as_string(),
        Err(e) => {
            web_sys::console::warn_1(&format!("keychain: {e}").into());
            None
        }
    }
}

/// Saves a secret. Returns whether the keychain accepted it.
pub async fn set(key: &str, secret: &str) -> bool {
    match tauri::invoke("secret_set", &Entry { key, secret }).await {
        Ok(_) => true,
        Err(e) => {
            web_sys::console::warn_1(&format!("keychain: {e}").into());
            false
        }
    }
}

pub fn delete(key: String) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = tauri::invoke("secret_delete", &Key { key: &key }).await {
            web_sys::console::warn_1(&format!("keychain: {e}").into());
        }
    });
}
