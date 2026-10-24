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
    pub username: &'static str,
    pub password: &'static str,
    pub shared: &'static str,
    pub connect: &'static str,
    pub connecting: &'static str,
    pub cancel: &'static str,
    pub advanced: &'static str,
    pub quality: &'static str,
    pub quality_lossless: &'static str,
    pub quality_high: &'static str,
    pub quality_low: &'static str,
    pub ssh_tunnel: &'static str,
    pub ssh_placeholder: &'static str,
    pub auto_reconnect: &'static str,
    pub recent: &'static str,
    pub favorite_add: &'static str,
    pub favorite_remove: &'static str,
    pub forget: &'static str,
    pub via: &'static str,
    pub auth_title: &'static str,
    pub auth_password_hint: &'static str,
    pub auth_credentials_hint: &'static str,
    pub auth_failed_hint: &'static str,
    pub ssh_auth_title: &'static str,
    pub ssh_auth_hint: &'static str,
    pub remember_password: &'static str,
    pub show_password: &'static str,
    pub hide_password: &'static str,
    pub keychain_error: &'static str,
    pub untrusted_cert: &'static str,
    pub ssh_unknown_key: &'static str,
    pub trust_and_connect: &'static str,
    pub retry: &'static str,
    pub connection_lost: &'static str,
    pub reconnecting_in: &'static str,
    pub reconnecting: &'static str,
    pub attempt: &'static str,
    pub reconnect_now: &'static str,
    pub keys: &'static str,
    pub keys_title: &'static str,
    pub ctrl_alt_del: &'static str,
    pub ctrl_alt_backspace: &'static str,
    pub ctrl_esc: &'static str,
    pub alt_tab: &'static str,
    pub alt_f4: &'static str,
    pub super_key: &'static str,
    pub print_screen: &'static str,
    pub escape: &'static str,
    pub function_keys: &'static str,
    pub clipboard_title: &'static str,
    pub zoom_title: &'static str,
    pub fit: &'static str,
    pub fit_title: &'static str,
    pub smooth: &'static str,
    pub remote_resize: &'static str,
    pub remote_resize_unsupported: &'static str,
    pub view_only: &'static str,
    pub view_only_title: &'static str,
    pub screenshot_title: &'static str,
    pub screenshot_saved: &'static str,
    pub refresh_title: &'static str,
    pub fullscreen_title: &'static str,
    pub exit_fullscreen_title: &'static str,
    pub fullscreen_hint: &'static str,
    pub new_window_title: &'static str,
    pub disconnect_title: &'static str,
    pub encrypted_tls: &'static str,
    pub encrypted_ssh: &'static str,
    pub not_encrypted: &'static str,
    pub fps: &'static str,
    pub stats_title: &'static str,
}

pub static FR: Texts = Texts {
    language: "Langue",
    subtitle: "Connexion à un bureau distant",
    server: "Serveur",
    server_placeholder: "hôte, hôte:affichage ou hôte::port",
    port: "Port par défaut",
    username: "Nom d'utilisateur",
    password: "Mot de passe",
    shared: "Session partagée (ne pas déconnecter les autres clients)",
    connect: "Se connecter",
    connecting: "Connexion…",
    cancel: "Annuler",
    advanced: "Options avancées",
    quality: "Qualité d'image",
    quality_lossless: "Sans perte (réseau local)",
    quality_high: "Bonne (JPEG, compromis)",
    quality_low: "Économique (connexion lente)",
    ssh_tunnel: "Tunnel SSH",
    ssh_placeholder: "utilisateur@passerelle[:port] (vide : connexion directe)",
    auto_reconnect: "Se reconnecter automatiquement si la connexion est perdue",
    recent: "Connexions récentes",
    favorite_add: "Ajouter aux favoris",
    favorite_remove: "Retirer des favoris",
    forget: "Oublier ce serveur",
    via: "via",
    auth_title: "Authentification",
    auth_password_hint: "Le serveur demande un mot de passe.",
    auth_credentials_hint: "Le serveur demande un nom d'utilisateur et un mot de passe.",
    auth_failed_hint: "Identifiants refusés, réessayez.",
    ssh_auth_title: "Authentification SSH",
    ssh_auth_hint: "Ni l'agent SSH ni les clés de ~/.ssh n'ont été acceptés. Mot de passe pour",
    remember_password: "Mémoriser dans le trousseau du système",
    show_password: "Afficher le mot de passe",
    hide_password: "Masquer le mot de passe",
    keychain_error: "Le mot de passe n'a pas pu être enregistré dans le trousseau du système",
    untrusted_cert: "Le certificat du serveur n'est signé par aucune autorité connue. S'il s'agit bien de votre serveur, vérifiez son empreinte SHA-256 :",
    ssh_unknown_key: "Cette passerelle SSH est inconnue (absente de ~/.ssh/known_hosts). Vérifiez l'empreinte de sa clé :",
    trust_and_connect: "Faire confiance et se connecter",
    retry: "Réessayer",
    connection_lost: "Connexion perdue",
    reconnecting_in: "Nouvelle tentative dans {} s",
    reconnecting: "Reconnexion…",
    attempt: "essai",
    reconnect_now: "Reconnecter maintenant",
    keys: "Touches",
    keys_title: "Envoyer une combinaison de touches",
    ctrl_alt_del: "Ctrl+Alt+Suppr",
    ctrl_alt_backspace: "Ctrl+Alt+Retour arrière",
    ctrl_esc: "Ctrl+Échap",
    alt_tab: "Alt+Tab",
    alt_f4: "Alt+F4",
    super_key: "Touche Windows / Super",
    print_screen: "Impr. écran",
    escape: "Échap",
    function_keys: "Touches de fonction",
    clipboard_title: "Envoyer le presse-papiers local au serveur",
    zoom_title: "Zoom et affichage",
    fit: "Ajuster",
    fit_title: "Adapter l'écran distant à la fenêtre",
    smooth: "Lissage de l'image",
    remote_resize: "Adapter la résolution distante à la fenêtre",
    remote_resize_unsupported: "(non pris en charge par ce serveur)",
    view_only: "Lecture seule",
    view_only_title: "Lecture seule : ne pas envoyer le clavier ni la souris",
    screenshot_title: "Enregistrer une capture d'écran",
    screenshot_saved: "Capture enregistrée",
    refresh_title: "Redemander tout l'écran",
    fullscreen_title: "Plein écran (Ctrl+Alt+Entrée)",
    exit_fullscreen_title: "Quitter le plein écran (Ctrl+Alt+Entrée)",
    fullscreen_hint: "Ctrl+Alt+Entrée pour quitter le plein écran · F8 pour la barre d'outils",
    new_window_title: "Nouvelle fenêtre (autre connexion)",
    disconnect_title: "Se déconnecter",
    encrypted_tls: "Connexion chiffrée (TLS)",
    encrypted_ssh: "Connexion chiffrée (tunnel SSH)",
    not_encrypted: "Connexion non chiffrée : l'écran et le clavier circulent en clair sur le réseau",
    fps: "i/s",
    stats_title: "Images par seconde et débit reçu",
};

pub static EN: Texts = Texts {
    language: "Language",
    subtitle: "Connect to a remote desktop",
    server: "Server",
    server_placeholder: "host, host:display or host::port",
    port: "Default port",
    username: "User name",
    password: "Password",
    shared: "Shared session (keep other clients connected)",
    connect: "Connect",
    connecting: "Connecting…",
    cancel: "Cancel",
    advanced: "Advanced options",
    quality: "Image quality",
    quality_lossless: "Lossless (local network)",
    quality_high: "Good (JPEG, balanced)",
    quality_low: "Low bandwidth (slow link)",
    ssh_tunnel: "SSH tunnel",
    ssh_placeholder: "user@gateway[:port] (empty: direct connection)",
    auto_reconnect: "Reconnect automatically when the connection is lost",
    recent: "Recent connections",
    favorite_add: "Add to favourites",
    favorite_remove: "Remove from favourites",
    forget: "Forget this server",
    via: "via",
    auth_title: "Authentication",
    auth_password_hint: "The server asks for a password.",
    auth_credentials_hint: "The server asks for a user name and a password.",
    auth_failed_hint: "Credentials rejected, try again.",
    ssh_auth_title: "SSH authentication",
    ssh_auth_hint: "Neither the SSH agent nor the keys in ~/.ssh were accepted. Password for",
    remember_password: "Remember in the system keychain",
    show_password: "Show password",
    hide_password: "Hide password",
    keychain_error: "The password could not be saved in the system keychain",
    untrusted_cert: "The server's certificate is not signed by a known authority. If this is your server, check its SHA-256 fingerprint:",
    ssh_unknown_key: "This SSH gateway is unknown (not in ~/.ssh/known_hosts). Check its key fingerprint:",
    trust_and_connect: "Trust and connect",
    retry: "Retry",
    connection_lost: "Connection lost",
    reconnecting_in: "Retrying in {} s",
    reconnecting: "Reconnecting…",
    attempt: "attempt",
    reconnect_now: "Reconnect now",
    keys: "Keys",
    keys_title: "Send a key combination",
    ctrl_alt_del: "Ctrl+Alt+Del",
    ctrl_alt_backspace: "Ctrl+Alt+Backspace",
    ctrl_esc: "Ctrl+Esc",
    alt_tab: "Alt+Tab",
    alt_f4: "Alt+F4",
    super_key: "Windows / Super key",
    print_screen: "Print Screen",
    escape: "Esc",
    function_keys: "Function keys",
    clipboard_title: "Send the local clipboard to the server",
    zoom_title: "Zoom and display",
    fit: "Fit",
    fit_title: "Scale the remote screen to the window",
    smooth: "Smooth scaling",
    remote_resize: "Resize the remote desktop to the window",
    remote_resize_unsupported: "(not supported by this server)",
    view_only: "View only",
    view_only_title: "View only: don't send keyboard and mouse",
    screenshot_title: "Save a screenshot",
    screenshot_saved: "Screenshot saved",
    refresh_title: "Request the whole screen again",
    fullscreen_title: "Full screen (Ctrl+Alt+Enter)",
    exit_fullscreen_title: "Exit full screen (Ctrl+Alt+Enter)",
    fullscreen_hint: "Ctrl+Alt+Enter to exit full screen · F8 for the toolbar",
    new_window_title: "New window (another connection)",
    disconnect_title: "Disconnect",
    encrypted_tls: "Encrypted connection (TLS)",
    encrypted_ssh: "Encrypted connection (SSH tunnel)",
    not_encrypted: "Unencrypted connection: screen and keyboard travel in clear over the network",
    fps: "fps",
    stats_title: "Frames per second and received bandwidth",
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
    /// Code of a backend error.
    pub fn code(&self) -> Option<&str> {
        match self {
            UiError::Backend(info) => Some(&info.code),
            _ => None,
        }
    }

    /// A network failure worth retrying automatically.
    pub fn is_transient(&self) -> bool {
        matches!(
            self.code(),
            Some("serverClosed" | "timeout" | "refused" | "network" | "connectionLost")
        )
    }

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
            UiError::Address(AddressError::MissingUser) => pick(
                "tunnel SSH : indiquez utilisateur@passerelle",
                "SSH tunnel: enter user@gateway",
            ),
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
        "credentialsRequired" => (
            "ce serveur exige un nom d'utilisateur et un mot de passe",
            "this server requires a user name and a password",
        ),
        "unsupportedSecurity" => (
            "aucun type de sécurité compatible (proposés : {})",
            "no compatible security type (offered: {})",
        ),
        "untrustedCertificate" => (
            "certificat du serveur non reconnu",
            "unrecognised server certificate",
        ),
        "tls" => ("erreur TLS : {}", "TLS error: {}"),
        "cancelled" => ("connexion annulée", "connection cancelled"),
        "ssh" => ("erreur SSH : {}", "SSH error: {}"),
        "sshAuthFailed" => ("authentification SSH refusée", "SSH authentication failed"),
        "sshHostKeyUnknown" => ("clé d'hôte SSH inconnue", "unknown SSH host key"),
        "sshHostKeyChanged" => (
            "ATTENTION : la clé d'hôte SSH a changé ({}). Il peut s'agir d'une attaque ; si le changement est légitime, retirez l'ancienne clé de ~/.ssh/known_hosts",
            "WARNING: the SSH host key changed ({}). This may be an attack; if the change is legitimate, remove the old key from ~/.ssh/known_hosts",
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
            "credentialsRequired",
            "untrustedCertificate",
            "tls",
            "cancelled",
            "ssh",
            "sshAuthFailed",
            "sshHostKeyUnknown",
            "sshHostKeyChanged",
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
