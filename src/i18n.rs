//! User interface languages (French / English).
//!
//! Every UI string lives in a [`Texts`] table, so a missing translation is a
//! compile error. Errors are kept structured ([`UiError`]) and translated when
//! displayed, so switching language also translates an error already shown.

use serde::{Deserialize, Serialize};

use crate::address::AddressError;
use crate::wire::ErrorInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Fr, Lang::En];

    /// BCP 47 tag, for `<html lang>`.
    pub fn tag(self) -> &'static str {
        match self {
            Lang::Fr => "fr",
            Lang::En => "en",
        }
    }

    /// Short label shown on the language switch.
    pub fn short(self) -> &'static str {
        match self {
            Lang::Fr => "FR",
            Lang::En => "EN",
        }
    }

    /// Name of the language in that language.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Fr => "Français",
            Lang::En => "English",
        }
    }

    /// French for French-speaking systems, English otherwise.
    pub fn from_locale(locale: &str) -> Lang {
        if locale.to_ascii_lowercase().starts_with("fr") {
            Lang::Fr
        } else {
            Lang::En
        }
    }

    /// Language of the system (the webview's `navigator.language`).
    pub fn detect() -> Lang {
        web_sys::window()
            .and_then(|w| w.navigator().language())
            .map_or(Lang::En, |l| Lang::from_locale(&l))
    }

    pub fn texts(self) -> &'static Texts {
        match self {
            Lang::Fr => &FR,
            Lang::En => &EN,
        }
    }
}

pub struct Texts {
    pub language: &'static str,
    pub subtitle: &'static str,
    pub server: &'static str,
    pub server_placeholder: &'static str,
    pub port: &'static str,
    pub password: &'static str,
    pub password_placeholder: &'static str,
    pub shared: &'static str,
    pub connect: &'static str,
    pub connecting: &'static str,
    pub ctrl_alt_del: &'static str,
    pub ctrl_alt_del_title: &'static str,
    pub clipboard: &'static str,
    pub clipboard_title: &'static str,
    pub fit: &'static str,
    pub fit_title: &'static str,
    pub fullscreen: &'static str,
    pub fullscreen_title: &'static str,
    pub refresh: &'static str,
    pub refresh_title: &'static str,
    pub disconnect: &'static str,
}

pub static FR: Texts = Texts {
    language: "Langue",
    subtitle: "Connexion à un bureau distant",
    server: "Serveur",
    server_placeholder: "hôte, hôte:affichage ou hôte::port",
    port: "Port",
    password: "Mot de passe",
    password_placeholder: "facultatif",
    shared: "Session partagée (ne pas déconnecter les autres clients)",
    connect: "Se connecter",
    connecting: "Connexion…",
    ctrl_alt_del: "Ctrl+Alt+Suppr",
    ctrl_alt_del_title: "Envoyer Ctrl+Alt+Suppr",
    clipboard: "Presse-papiers",
    clipboard_title: "Envoyer le presse-papiers local au serveur",
    fit: "Ajuster",
    fit_title: "Adapter l'écran distant à la fenêtre",
    fullscreen: "Plein écran",
    fullscreen_title: "Basculer en plein écran",
    refresh: "Rafraîchir",
    refresh_title: "Redemander tout l'écran",
    disconnect: "Déconnecter",
};

pub static EN: Texts = Texts {
    language: "Language",
    subtitle: "Connect to a remote desktop",
    server: "Server",
    server_placeholder: "host, host:display or host::port",
    port: "Port",
    password: "Password",
    password_placeholder: "optional",
    shared: "Shared session (keep other clients connected)",
    connect: "Connect",
    connecting: "Connecting…",
    ctrl_alt_del: "Ctrl+Alt+Del",
    ctrl_alt_del_title: "Send Ctrl+Alt+Del",
    clipboard: "Clipboard",
    clipboard_title: "Send the local clipboard to the server",
    fit: "Fit",
    fit_title: "Scale the remote screen to the window",
    fullscreen: "Full screen",
    fullscreen_title: "Toggle full screen",
    refresh: "Refresh",
    refresh_title: "Request the whole screen again",
    disconnect: "Disconnect",
};

/// An error to show on the connection screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiError {
    /// Reported by the backend.
    Backend(ErrorInfo),
    Address(AddressError),
    InvalidPort,
    /// The server closed the session without giving a reason.
    SessionEnded,
    /// Unexpected technical failure (shown as is).
    Internal(String),
}

impl UiError {
    pub fn message(&self, lang: Lang) -> String {
        let fr = lang == Lang::Fr;
        let pick = |f: &str, e: &str| if fr { f.to_string() } else { e.to_string() };
        match self {
            UiError::Backend(info) => backend_message(info, lang),
            UiError::Address(AddressError::Empty) => {
                pick("indiquez l'adresse du serveur", "enter the server address")
            }
            UiError::Address(AddressError::InvalidPort(p)) => {
                if fr {
                    format!("port invalide : « {p} »")
                } else {
                    format!("invalid port: “{p}”")
                }
            }
            UiError::Address(AddressError::BadIpv6) => {
                pick("adresse IPv6 mal formée", "malformed IPv6 address")
            }
            UiError::InvalidPort => pick("port invalide", "invalid port"),
            UiError::SessionEnded => pick(
                "le serveur a mis fin à la session",
                "the server ended the session",
            ),
            UiError::Internal(e) => e.clone(),
        }
    }
}

fn backend_message(info: &ErrorInfo, lang: Lang) -> String {
    let fr = lang == Lang::Fr;
    let d = info.detail.as_deref().unwrap_or("");
    // (French, English); `{}` is replaced by the detail.
    let (f, e) = match info.code.as_str() {
        "serverClosed" => (
            "le serveur a fermé la connexion",
            "the server closed the connection",
        ),
        "timeout" => ("délai de connexion dépassé", "connection timed out"),
        "refused" => (
            "connexion refusée (aucun serveur VNC à cette adresse ?)",
            "connection refused (no VNC server at this address?)",
        ),
        "network" => ("erreur réseau : {}", "network error: {}"),
        "notVnc" => ("ce n'est pas un serveur VNC ({})", "not a VNC server ({})"),
        "serverRefused" => (
            "le serveur a refusé la connexion : {}",
            "the server refused the connection: {}",
        ),
        "authFailed" if info.detail.is_some() => {
            ("authentification refusée : {}", "authentication failed: {}")
        }
        "authFailed" => ("mot de passe incorrect", "wrong password"),
        "passwordRequired" => (
            "ce serveur exige un mot de passe",
            "this server requires a password",
        ),
        "unsupportedSecurity" => (
            "aucun type de sécurité compatible (proposés : {} ; seuls « aucun » et « VNC » sont gérés)",
            "no compatible security type (offered: {}; only “None” and “VNC” are supported)",
        ),
        "unsupportedEncoding" => (
            "encodage non pris en charge : {}",
            "unsupported encoding: {}",
        ),
        "screenTooLarge" => (
            "écran distant trop grand ({})",
            "remote screen too large ({})",
        ),
        "unknownMessage" => (
            "message serveur inconnu (type {})",
            "unknown server message (type {})",
        ),
        "malformed" => (
            "données invalides reçues du serveur ({})",
            "invalid data received from the server ({})",
        ),
        "connectionLost" => ("connexion interrompue", "connection lost"),
        code => {
            let msg = if fr { "erreur" } else { "error" };
            return match (&info.detail, fr) {
                (Some(d), true) => format!("{msg} {code} : {d}"),
                (Some(d), false) => format!("{msg} {code}: {d}"),
                (None, _) => format!("{msg} {code}"),
            };
        }
    };
    (if fr { f } else { e }).replace("{}", d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend(code: &str, detail: Option<&str>) -> UiError {
        UiError::Backend(ErrorInfo {
            code: code.into(),
            detail: detail.map(Into::into),
        })
    }

    #[test]
    fn locale_detection() {
        assert_eq!(Lang::from_locale("fr-FR"), Lang::Fr);
        assert_eq!(Lang::from_locale("FR"), Lang::Fr);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("de"), Lang::En);
    }

    #[test]
    fn backend_errors_are_translated() {
        let e = backend("authFailed", None);
        assert_eq!(e.message(Lang::Fr), "mot de passe incorrect");
        assert_eq!(e.message(Lang::En), "wrong password");
        let e = backend("authFailed", Some("too many attempts"));
        assert_eq!(
            e.message(Lang::En),
            "authentication failed: too many attempts"
        );
        let e = backend("unknownMessage", Some("42"));
        assert_eq!(e.message(Lang::Fr), "message serveur inconnu (type 42)");
        assert_eq!(backend("mystery", None).message(Lang::En), "error mystery");
    }

    #[test]
    fn every_backend_code_has_a_translation() {
        // Codes produced by `rfb::Error::info` in the backend.
        for code in [
            "serverClosed",
            "timeout",
            "refused",
            "network",
            "notVnc",
            "serverRefused",
            "authFailed",
            "passwordRequired",
            "unsupportedSecurity",
            "unsupportedEncoding",
            "screenTooLarge",
            "unknownMessage",
            "malformed",
            "connectionLost",
        ] {
            for lang in Lang::ALL {
                let msg = backend(code, Some("x")).message(lang);
                let fallback = backend("mystery", Some("x"))
                    .message(lang)
                    .replace("mystery", code);
                assert_ne!(msg, fallback, "{code} has no translation");
            }
        }
    }

    #[test]
    fn local_errors_are_translated() {
        let e = UiError::Address(AddressError::InvalidPort("x".into()));
        assert_eq!(e.message(Lang::Fr), "port invalide : « x »");
        assert_eq!(e.message(Lang::En), "invalid port: “x”");
    }
}
