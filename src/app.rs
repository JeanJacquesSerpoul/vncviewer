//! Application state and connection life cycle.

use std::time::Duration;

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use crate::connect::ConnectScreen;
use crate::i18n::{Lang, Texts, UiError};
use crate::prefs::{self, Prefs, Server};
use crate::toolbar::Viewer;
use crate::viewer::{self, ConnectParams, SshParams};
use crate::wire::Event;
use crate::{address, secrets, tauri};

/// Automatic reconnection attempts before giving up.
const MAX_ATTEMPTS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Idle,
    Connecting,
    Connected,
    /// The connection was lost; attempt `attempt` starts at `at` (ms).
    Reconnecting {
        attempt: u32,
        at: f64,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Desktop {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecurityInfo {
    pub encrypted: bool,
    pub tunnel: bool,
}

/// Credentials asked for after the server refused to go on without them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    Vnc { need_user: bool, failed: bool },
    Ssh { failed: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Keys,
    Zoom,
}

/// What to connect to, and with what (kept for retries and reconnections).
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub server: Server,
    pub password: Option<String>,
    pub ssh_password: Option<String>,
    /// SSH host key fingerprint accepted by the user for this attempt.
    pub trusted_ssh_key: Option<String>,
}

impl Target {
    pub fn vnc_key(&self) -> String {
        let s = &self.server;
        secrets::vnc_key(&s.address, &s.username, &s.ssh)
    }

    pub fn ssh_key(&self) -> String {
        secrets::ssh_key(&self.server.ssh)
    }
}

/// Shared UI state. Every field is a signal, so the whole thing is `Copy`.
#[derive(Clone, Copy)]
pub struct Ctx {
    pub lang: RwSignal<Lang>,
    pub prefs: RwSignal<Prefs>,
    pub phase: RwSignal<Phase>,
    pub error: RwSignal<Option<UiError>>,
    pub prompt: RwSignal<Option<Prompt>>,
    pub desktop: RwSignal<Desktop>,
    pub security: RwSignal<Option<SecurityInfo>>,
    /// The server accepts desktop size requests.
    pub remote_resize_ok: RwSignal<bool>,
    pub fullscreen: RwSignal<bool>,
    /// Toolbar shown over the screen in full screen.
    pub toolbar_shown: RwSignal<bool>,
    pub view_only: RwSignal<bool>,
    pub menu: RwSignal<Option<Menu>>,
    pub toast: RwSignal<Option<(u32, String)>>,
    /// Frames and bytes received during the last second.
    pub stats: RwSignal<(u32, u64)>,
    /// Current time (ms), updated twice a second.
    pub now: RwSignal<f64>,
    pub bell: RwSignal<u32>,
    pub canvas: NodeRef<html::Canvas>,
    pub screen: NodeRef<html::Div>,
    // Connection form.
    pub host: RwSignal<String>,
    pub port: RwSignal<String>,
    pub ssh: RwSignal<String>,
    pub username: RwSignal<String>,
    pub current: StoredValue<Option<Target>>,
}

impl Ctx {
    pub fn t(&self) -> &'static Texts {
        self.lang.get().texts()
    }

    pub fn focus_canvas(&self) {
        if let Some(c) = self.canvas.get_untracked() {
            let _ = c.focus();
        }
    }

    pub fn show_toast(&self, text: String) {
        let id = self.toast.get_untracked().map_or(0, |(id, _)| id + 1);
        self.toast.set(Some((id, text)));
        let toast = self.toast;
        set_timeout(
            move || {
                if toast.get_untracked().is_some_and(|(i, _)| i == id) {
                    toast.set(None);
                }
            },
            Duration::from_millis(3500),
        );
    }

    /// The form as a target, reusing what the history knows about it.
    pub fn form_target(&self) -> Target {
        let mut server = Server {
            address: self.host.get_untracked().trim().to_owned(),
            ssh: self.ssh.get_untracked().trim().to_owned(),
            username: self.username.get_untracked().trim().to_owned(),
            ..Default::default()
        };
        if let Some(known) = self
            .prefs
            .with_untracked(|p| p.servers.iter().find(|s| s.same_target(&server)).cloned())
        {
            server.remember = known.remember;
            server.favorite = known.favorite;
            server.name = known.name;
        }
        Target {
            server,
            ..Default::default()
        }
    }

    pub fn fill_form(&self, s: &Server) {
        self.host.set(s.address.clone());
        self.ssh.set(s.ssh.clone());
        self.username.set(s.username.clone());
    }
}

pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Starts connecting to `target`.
pub fn start(ctx: Ctx, mut target: Target) {
    let p = ctx.prefs.get_untracked();
    let fail = |e| {
        ctx.phase.set(Phase::Idle);
        ctx.error.set(Some(e));
    };
    let (host, port) = match address::resolve(&target.server.address, p.port) {
        Ok(t) => t,
        Err(e) => return fail(UiError::Address(e)),
    };
    let ssh = match target.server.ssh.as_str() {
        "" => None,
        s => match address::parse_ssh(s) {
            Ok(t) => Some(t),
            Err(e) => return fail(UiError::Address(e)),
        },
    };
    let Some(canvas) = ctx.canvas.get_untracked() else {
        return;
    };
    ctx.prefs.update(|prefs| {
        prefs.host = target.server.address.clone();
        prefs.ssh = target.server.ssh.clone();
        prefs.username = target.server.username.clone();
    });
    ctx.error.set(None);
    ctx.prompt.set(None);
    if !matches!(ctx.phase.get_untracked(), Phase::Reconnecting { .. }) {
        ctx.phase.set(Phase::Connecting);
    }
    spawn_local(async move {
        if target.server.remember {
            if target.password.is_none() {
                target.password = secrets::get(&target.vnc_key()).await;
            }
            if ssh.is_some() && target.ssh_password.is_none() {
                target.ssh_password = secrets::get(&target.ssh_key()).await;
            }
        }
        ctx.current.set_value(Some(target.clone()));
        let params = ConnectParams {
            trusted_cert: p.trusted_certs.get(&format!("{host}:{port}")).cloned(),
            host,
            port,
            username: Some(target.server.username.clone()).filter(|u| !u.is_empty()),
            password: target.password.clone(),
            shared: p.shared,
            quality: p.quality.key(),
            ssh: ssh.map(|t| SshParams {
                host: t.host,
                port: t.port,
                user: t.user,
                password: target.ssh_password.clone(),
                trusted_key: target.trusted_ssh_key.clone(),
            }),
        };
        match viewer::connect(params, canvas, move |ev| on_event(ctx, ev)).await {
            Ok(()) => {
                set_connected(ctx);
                remember(ctx, &target);
            }
            Err(e) => failed(ctx, e, &target),
        }
    });
}

/// Saves a successful connection to the history (and its passwords, if asked).
fn remember(ctx: Ctx, target: &Target) {
    let mut server = target.server.clone();
    server.last_used = now_ms();
    let name = ctx.desktop.with_untracked(|d| d.name.clone());
    if !name.is_empty() {
        server.name = name;
    }
    if server.remember {
        let secrets = [
            (target.vnc_key(), target.password.clone()),
            (target.ssh_key(), target.ssh_password.clone()),
        ];
        let saved = server.clone();
        spawn_local(async move {
            for (key, secret) in secrets {
                if let Some(secret) = secret
                    && !secrets::set(&key, &secret).await
                {
                    // Not saved after all: don't claim it is.
                    ctx.prefs.update(|p| {
                        if let Some(s) = p.servers.iter_mut().find(|s| s.same_target(&saved)) {
                            s.remember = false;
                        }
                    });
                    let msg = ctx.lang.get_untracked().texts().keychain_error;
                    return ctx.show_toast(msg.into());
                }
            }
        });
    }
    ctx.prefs.update(|p| p.remember_server(server));
}

fn failed(ctx: Ctx, e: UiError, target: &Target) {
    let reconnecting = match ctx.phase.get_untracked() {
        Phase::Reconnecting { attempt, .. } => Some(attempt),
        _ => None,
    };
    let need_user = !target.server.username.is_empty();
    let prompt = match e.code() {
        Some("cancelled") => {
            if ctx.phase.get_untracked() != Phase::Connected {
                ctx.phase.set(Phase::Idle);
            }
            return;
        }
        Some("passwordRequired") => Some(Prompt::Vnc {
            need_user,
            failed: false,
        }),
        Some("credentialsRequired") => Some(Prompt::Vnc {
            need_user: true,
            failed: false,
        }),
        Some("authFailed") => Some(Prompt::Vnc {
            need_user,
            failed: true,
        }),
        Some("sshAuthFailed") => Some(Prompt::Ssh {
            failed: target.ssh_password.is_some(),
        }),
        _ => None,
    };
    if let (Some(attempt), None, true) = (reconnecting, prompt, e.is_transient()) {
        ctx.error.set(Some(e));
        return schedule_reconnect(ctx, attempt + 1);
    }
    ctx.phase.set(Phase::Idle);
    ctx.error.set(Some(e));
    ctx.prompt.set(prompt);
}

fn set_connected(ctx: Ctx) {
    if ctx.phase.get_untracked() != Phase::Connected {
        ctx.phase.set(Phase::Connected);
        ctx.error.set(None);
        // After the view is shown (a hidden canvas can't take the focus).
        set_timeout(move || ctx.focus_canvas(), Duration::ZERO);
    }
}

fn on_event(ctx: Ctx, ev: Event) {
    match ev {
        Event::Connected {
            encrypted, tunnel, ..
        } => {
            ctx.security.set(Some(SecurityInfo { encrypted, tunnel }));
            ctx.remote_resize_ok.set(false);
        }
        Event::RemoteResize => ctx.remote_resize_ok.set(true),
        Event::Resize {
            width,
            height,
            name,
        } => {
            let renamed = ctx.desktop.with_untracked(|d| d.name != name);
            ctx.desktop.set(crate::app::Desktop {
                name: name.clone(),
                width,
                height,
            });
            if renamed && let Some(t) = ctx.current.get_value() {
                ctx.prefs.update(|p| {
                    if let Some(s) = p.servers.iter_mut().find(|s| s.same_target(&t.server)) {
                        s.name = name;
                    }
                });
            }
            set_connected(ctx);
        }
        Event::Bell => ctx.bell.update(|n| *n += 1),
        Event::Disconnected { reason } => {
            viewer::release_all();
            let e = reason.map_or(UiError::SessionEnded, UiError::Backend);
            let auto = ctx.prefs.with_untracked(|p| p.auto_reconnect);
            if ctx.phase.get_untracked() == Phase::Connected && auto && e.is_transient() {
                ctx.error.set(Some(e));
                schedule_reconnect(ctx, 1);
            } else {
                ctx.phase.set(Phase::Idle);
                ctx.error.set(Some(e));
            }
        }
    }
}

fn schedule_reconnect(ctx: Ctx, attempt: u32) {
    if attempt > MAX_ATTEMPTS {
        ctx.phase.set(Phase::Idle);
        return;
    }
    let delay = 2u64.pow(attempt - 1).min(30);
    let at = now_ms() + delay as f64 * 1000.0;
    let phase = Phase::Reconnecting { attempt, at };
    ctx.phase.set(phase);
    set_timeout(
        move || {
            if ctx.phase.get_untracked() == phase
                && let Some(t) = ctx.current.get_value()
            {
                start(ctx, t);
            }
        },
        Duration::from_secs(delay),
    );
}

pub fn reconnect_now(ctx: Ctx) {
    if let Some(t) = ctx.current.get_value() {
        start(ctx, t);
    }
}

pub fn cancel(ctx: Ctx) {
    viewer::disconnect();
    ctx.phase.set(Phase::Idle);
}

pub fn disconnect(ctx: Ctx) {
    viewer::disconnect();
    ctx.phase.set(Phase::Idle);
    ctx.error.set(None);
    ctx.security.set(None);
    ctx.view_only.set(false);
    viewer::set_view_only(false);
}

/// Asks the server for a desktop the size of the screen area, if enabled.
pub fn sync_remote_size(ctx: Ctx) {
    let enabled = ctx.prefs.with_untracked(|p| p.remote_resize);
    if !enabled
        || !ctx.remote_resize_ok.get_untracked()
        || ctx.phase.get_untracked() != Phase::Connected
    {
        return;
    }
    let Some(screen) = ctx.screen.get_untracked() else {
        return;
    };
    let (w, h) = (screen.client_width(), screen.client_height());
    let same = ctx
        .desktop
        .with_untracked(|d| d.width as i32 == w && d.height as i32 == h);
    if w < 64 || h < 64 || same {
        return;
    }
    #[derive(Serialize)]
    struct Size {
        width: u16,
        height: u16,
    }
    tauri::invoke_detached_with(
        "set_desktop_size",
        Size {
            width: w.min(u16::MAX as i32) as u16,
            height: h.min(u16::MAX as i32) as u16,
        },
    );
}

pub fn set_fullscreen_state(ctx: Ctx, on: bool) {
    if on && !ctx.fullscreen.get_untracked() {
        ctx.toolbar_shown.set(false);
        ctx.show_toast(ctx.lang.get_untracked().texts().fullscreen_hint.into());
    }
    ctx.fullscreen.set(on);
}

pub fn toggle_fullscreen(ctx: Ctx) {
    spawn_local(async move {
        match tauri::invoke_value("toggle_fullscreen", wasm_bindgen::JsValue::UNDEFINED).await {
            Ok(v) => set_fullscreen_state(ctx, v.as_bool().unwrap_or(false)),
            Err(e) => web_sys::console::warn_1(&format!("fullscreen: {e}").into()),
        }
        ctx.focus_canvas();
    });
}

#[component]
pub fn App() -> impl IntoView {
    let saved = prefs::load();
    let lang = RwSignal::new(saved.lang.unwrap_or_else(Lang::detect));
    let ctx = Ctx {
        lang,
        host: RwSignal::new(saved.host.clone()),
        port: RwSignal::new(saved.port.to_string()),
        ssh: RwSignal::new(saved.ssh.clone()),
        username: RwSignal::new(saved.username.clone()),
        prefs: RwSignal::new(saved),
        phase: RwSignal::new(Phase::Idle),
        error: RwSignal::new(None),
        prompt: RwSignal::new(None),
        desktop: RwSignal::new(Desktop::default()),
        security: RwSignal::new(None),
        remote_resize_ok: RwSignal::new(false),
        fullscreen: RwSignal::new(false),
        toolbar_shown: RwSignal::new(false),
        view_only: RwSignal::new(false),
        menu: RwSignal::new(None),
        toast: RwSignal::new(None),
        stats: RwSignal::new((0, 0)),
        now: RwSignal::new(now_ms()),
        bell: RwSignal::new(0),
        canvas: NodeRef::new(),
        screen: NodeRef::new(),
        current: StoredValue::new(None),
    };

    // Settings are saved whenever they change.
    Effect::new(move |_| ctx.prefs.with(prefs::save));
    // Document language, for accessibility and hyphenation.
    Effect::new(move |_| {
        let tag = lang.get().tag();
        if let Some(root) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
        {
            let _ = root.set_attribute("lang", tag);
        }
    });
    // Remember an explicit choice (the first run only reflects the initial value).
    Effect::new(move |first: Option<()>| {
        let l = lang.get();
        if first.is_some() {
            ctx.prefs.update(|p| p.lang = Some(l));
        }
    });
    // Window title: the remote desktop's name while connected.
    let title = Memo::new(move |_| match ctx.phase.get() {
        Phase::Connected | Phase::Reconnecting { .. } => {
            let name = ctx.desktop.with(|d| d.name.clone());
            if name.is_empty() {
                "VNC Viewer".to_owned()
            } else {
                format!("{name} — VNC Viewer")
            }
        }
        _ => "VNC Viewer".to_owned(),
    });
    Effect::new(move |_| {
        #[derive(Serialize)]
        struct Title {
            title: String,
        }
        tauri::invoke_detached_with("set_title", Title { title: title.get() });
    });
    // Remote desktop size follows the window, when enabled.
    Effect::new(move |_| {
        ctx.remote_resize_ok.track();
        ctx.fullscreen.track();
        ctx.prefs.with(|p| p.remote_resize);
        if ctx.phase.get() == Phase::Connected {
            set_timeout(move || sync_remote_size(ctx), Duration::from_millis(50));
        }
    });
    let resize_seq = StoredValue::new(0u32);
    let _ = window_event_listener(leptos::ev::resize, move |_| {
        let n = resize_seq.get_value() + 1;
        resize_seq.set_value(n);
        set_timeout(
            move || {
                if resize_seq.get_value() == n {
                    sync_remote_size(ctx);
                }
            },
            Duration::from_millis(300),
        );
        // The window manager may leave full screen on its own.
        spawn_local(async move {
            if let Ok(v) =
                tauri::invoke_value("is_fullscreen", wasm_bindgen::JsValue::UNDEFINED).await
            {
                set_fullscreen_state(ctx, v.as_bool().unwrap_or(false));
            }
        });
    });
    let _ = set_interval_with_handle(move || ctx.now.set(now_ms()), Duration::from_millis(500));
    let _ = set_interval_with_handle(
        move || ctx.stats.set(viewer::take_stats()),
        Duration::from_secs(1),
    );

    view! {
        <Show when=move || {
            matches!(ctx.phase.get(), Phase::Idle | Phase::Connecting)
        }>
            <ConnectScreen ctx />
        </Show>
        <Viewer ctx />
        <Show when=move || ctx.toast.get().is_some()>
            <div class="toast" role="status">
                {move || ctx.toast.get().map(|(_, t)| t)}
            </div>
        </Show>
    }
}
