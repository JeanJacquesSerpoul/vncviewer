use leptos::ev::{KeyboardEvent, PointerEvent, SubmitEvent, WheelEvent};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use crate::i18n::{Lang, UiError};
use crate::prefs::{self, Prefs};
use crate::viewer::{self, ConnectParams};
use crate::wire::Event;
use crate::{address, tauri};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Desktop {
    name: String,
    width: u32,
    height: u32,
}

const CTRL_ALT_DEL: [u32; 3] = [0xffe3, 0xffe9, 0xffff];

fn canvas_of<E: AsRef<web_sys::Event>>(ev: &E) -> Option<HtmlCanvasElement> {
    ev.as_ref().current_target()?.dyn_into().ok()
}

/// FR / EN toggle. Doesn't take the focus, so the remote screen keeps the keyboard.
#[component]
fn LangSwitch(lang: RwSignal<Lang>) -> impl IntoView {
    view! {
        <div class="lang-switch" role="group" aria-label=move || lang.get().texts().language>
            {Lang::ALL
                .map(|l| {
                    view! {
                        <button
                            type="button"
                            lang=l.tag()
                            title=l.name()
                            class:active=move || lang.get() == l
                            aria-pressed=move || (lang.get() == l).to_string()
                            on:mousedown=|ev| ev.prevent_default()
                            on:click=move |_| lang.set(l)
                        >
                            {l.short()}
                        </button>
                    }
                })}
        </div>
    }
}

#[component]
pub fn App() -> impl IntoView {
    let saved = prefs::load();
    let lang = RwSignal::new(saved.lang.unwrap_or_else(Lang::detect));
    let lang_chosen = RwSignal::new(saved.lang.is_some());
    let t = move || lang.get().texts();
    let host = RwSignal::new(saved.host);
    let port = RwSignal::new(saved.port.to_string());
    let password = RwSignal::new(String::new());
    let shared = RwSignal::new(saved.shared);
    let fit = RwSignal::new(saved.fit);

    let phase = RwSignal::new(Phase::Idle);
    let error = RwSignal::new(None::<UiError>);
    let desktop = RwSignal::new(Desktop::default());
    let canvas_ref = NodeRef::<html::Canvas>::new();

    let save_prefs = move || {
        prefs::save(&Prefs {
            host: host.get_untracked(),
            port: port.get_untracked().trim().parse().unwrap_or(5900),
            shared: shared.get_untracked(),
            fit: fit.get_untracked(),
            // Keep following the system language until the user picks one.
            lang: lang_chosen.get_untracked().then(|| lang.get_untracked()),
        })
    };

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
        lang.track();
        if first.is_some() {
            lang_chosen.set(true);
            save_prefs();
        }
    });

    let focus_canvas = move || {
        if let Some(c) = canvas_ref.get_untracked() {
            let _ = c.focus();
        }
    };
    let set_connected = move || {
        if phase.get_untracked() != Phase::Connected {
            phase.set(Phase::Connected);
            // After the view is shown (a hidden canvas can't take the focus).
            set_timeout(focus_canvas, std::time::Duration::ZERO);
        }
    };

    let on_event = move |ev: Event| match ev {
        Event::Resize {
            width,
            height,
            name,
        } => {
            desktop.set(Desktop {
                name,
                width,
                height,
            });
            set_connected();
        }
        Event::Bell => {}
        Event::Disconnected { reason } => {
            viewer::release_all();
            phase.set(Phase::Idle);
            error.set(Some(reason.map_or(UiError::SessionEnded, UiError::Backend)));
        }
    };

    let connect = move |ev: SubmitEvent| {
        ev.prevent_default();
        if phase.get_untracked() == Phase::Connecting {
            return;
        }
        let default_port = match port.get_untracked().trim().parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => return error.set(Some(UiError::InvalidPort)),
        };
        let (host_name, port_num) = match address::resolve(&host.get_untracked(), default_port) {
            Ok(target) => target,
            Err(e) => return error.set(Some(UiError::Address(e))),
        };
        let Some(canvas) = canvas_ref.get_untracked() else {
            return;
        };
        save_prefs();
        error.set(None);
        phase.set(Phase::Connecting);
        let params = ConnectParams {
            host: host_name,
            port: port_num,
            password: Some(password.get_untracked()),
            shared: shared.get_untracked(),
        };
        spawn_local(async move {
            match viewer::connect(params, canvas, on_event).await {
                Ok(()) if phase.get_untracked() == Phase::Connecting => set_connected(),
                Ok(()) => {}
                Err(e) => {
                    phase.set(Phase::Idle);
                    error.set(Some(e));
                }
            }
        });
    };

    let disconnect = move |_| {
        viewer::disconnect();
        phase.set(Phase::Idle);
    };
    let ctrl_alt_del = move |_| {
        viewer::send_combo(&CTRL_ALT_DEL);
        focus_canvas();
    };
    let paste = move |_| {
        tauri::invoke_detached("send_clipboard");
        focus_canvas();
    };
    let toggle_fit = move |_| {
        fit.update(|f| *f = !*f);
        save_prefs();
        focus_canvas();
    };
    let fullscreen = move |_| {
        tauri::invoke_detached("toggle_fullscreen");
        focus_canvas();
    };
    let refresh = move |_| {
        tauri::invoke_detached("refresh");
        focus_canvas();
    };

    let connecting = move || phase.get() == Phase::Connecting;

    view! {
        <Show when=move || phase.get() != Phase::Connected>
            <div class="connect-screen">
                <form class="card" on:submit=connect>
                    <LangSwitch lang />
                    <div class="brand">
                        <svg viewBox="0 0 24 24" class="logo" aria-hidden="true">
                            <rect x="2.5" y="4" width="19" height="13" rx="2" />
                            <path d="M8 21h8M12 17v4" />
                        </svg>
                        <div>
                            <h1>"VNC Viewer"</h1>
                            <p class="subtitle">{move || t().subtitle}</p>
                        </div>
                    </div>

                    <label>
                        <span>{move || t().server}</span>
                        <input
                            type="text"
                            placeholder=move || t().server_placeholder
                            autofocus
                            spellcheck="false"
                            prop:value=host
                            on:input=move |ev| host.set(event_target_value(&ev))
                        />
                    </label>
                    <label>
                        <span>{move || t().port}</span>
                        <input
                            type="number"
                            min="1"
                            max="65535"
                            prop:value=port
                            on:input=move |ev| port.set(event_target_value(&ev))
                        />
                    </label>
                    <label>
                        <span>{move || t().password}</span>
                        <input
                            type="password"
                            placeholder=move || t().password_placeholder
                            prop:value=password
                            on:input=move |ev| password.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="check">
                        <input
                            type="checkbox"
                            prop:checked=shared
                            on:change=move |ev| shared.set(event_target_checked(&ev))
                        />
                        <span>{move || t().shared}</span>
                    </label>

                    <Show when=move || error.get().is_some()>
                        <p class="error" role="alert">
                            {move || error.get().map(|e| e.message(lang.get()))}
                        </p>
                    </Show>

                    <button type="submit" class="primary" disabled=connecting>
                        {move || if connecting() { t().connecting } else { t().connect }}
                    </button>
                </form>
            </div>
        </Show>

        <div class="viewer" class:hidden=move || phase.get() != Phase::Connected>
            <header class="toolbar">
                <div class="desktop-name">
                    <strong>{move || desktop.with(|d| d.name.clone())}</strong>
                    <span class="dims">
                        {move || desktop.with(|d| format!("{} × {}", d.width, d.height))}
                    </span>
                </div>
                <div class="actions">
                    <button on:click=ctrl_alt_del title=move || t().ctrl_alt_del_title>
                        {move || t().ctrl_alt_del}
                    </button>
                    <button on:click=paste title=move || t().clipboard_title>
                        {move || t().clipboard}
                    </button>
                    <button
                        on:click=toggle_fit
                        class:active=move || fit.get()
                        title=move || t().fit_title
                    >
                        {move || t().fit}
                    </button>
                    <button on:click=fullscreen title=move || t().fullscreen_title>
                        {move || t().fullscreen}
                    </button>
                    <button on:click=refresh title=move || t().refresh_title>
                        {move || t().refresh}
                    </button>
                    <button class="danger" on:click=disconnect>
                        {move || t().disconnect}
                    </button>
                </div>
                <LangSwitch lang />
            </header>
            <div class="screen" class:fit=move || fit.get()>
                <canvas
                    node_ref=canvas_ref
                    tabindex="0"
                    width="0"
                    height="0"
                    on:pointerdown=move |ev: PointerEvent| {
                        if let Some(c) = canvas_of(&ev) {
                            viewer::on_pointer_down(&ev, &c)
                        }
                    }
                    on:pointermove=move |ev: PointerEvent| {
                        if let Some(c) = canvas_of(&ev) {
                            viewer::on_pointer(&ev, &c)
                        }
                    }
                    on:pointerup=move |ev: PointerEvent| {
                        if let Some(c) = canvas_of(&ev) {
                            viewer::on_pointer(&ev, &c)
                        }
                    }
                    on:wheel=move |ev: WheelEvent| {
                        if let Some(c) = canvas_of(&ev) {
                            viewer::on_wheel(&ev, &c)
                        }
                    }
                    on:contextmenu=move |ev| ev.prevent_default()
                    on:keydown=move |ev: KeyboardEvent| viewer::on_key(&ev, true)
                    on:keyup=move |ev: KeyboardEvent| viewer::on_key(&ev, false)
                    on:blur=move |_| viewer::release_all()
                    on:focus=move |_| tauri::invoke_detached("send_clipboard")
                ></canvas>
            </div>
        </div>
    }
}
