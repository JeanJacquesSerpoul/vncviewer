//! Remote screen view: toolbar, menus, canvas and reconnection overlay.

use std::time::Duration;

use leptos::ev::{KeyboardEvent, PointerEvent, WheelEvent};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use crate::app::{self, Ctx, Menu, Phase};
use crate::connect::LangSwitch;
use crate::i18n::{Lang, Texts};
use crate::icons::{Icon, Svg};
use crate::prefs::{FIT, ZOOMS};
use crate::tauri;
use crate::viewer::{self, Hotkey};

const CTRL: u32 = 0xffe3;
const ALT: u32 = 0xffe9;

/// Key combinations of the Keys menu.
fn combos(t: &Texts) -> [(&'static str, &'static [u32]); 8] {
    [
        (t.ctrl_alt_del, &[CTRL, ALT, 0xffff]),
        (t.ctrl_alt_backspace, &[CTRL, ALT, 0xff08]),
        (t.ctrl_esc, &[CTRL, 0xff1b]),
        (t.alt_tab, &[ALT, 0xff09]),
        (t.alt_f4, &[ALT, 0xffc1]),
        (t.super_key, &[0xffeb]),
        (t.print_screen, &[0xff61]),
        (t.escape, &[0xff1b]),
    ]
}

fn canvas_of<E: AsRef<web_sys::Event>>(ev: &E) -> Option<HtmlCanvasElement> {
    ev.as_ref().current_target()?.dyn_into().ok()
}

/// "850 Ko/s", "1,2 Mo/s".
fn format_rate(bytes: u64, lang: Lang) -> String {
    let (k, m) = match lang {
        Lang::Fr => ("Ko/s", "Mo/s"),
        Lang::En => ("kB/s", "MB/s"),
    };
    let s = if bytes >= 1_000_000 {
        format!("{:.1} {m}", bytes as f64 / 1e6)
    } else {
        format!("{} {k}", bytes / 1000)
    };
    if lang == Lang::Fr {
        s.replace('.', ",")
    } else {
        s
    }
}

/// `name-2026-10-24_08-30-12.png`, with only safe characters.
fn screenshot_name(desktop: &str) -> String {
    let d = js_sys::Date::new_0();
    let base: String = desktop
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-');
    format!(
        "{}-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.png",
        if base.is_empty() { "vnc" } else { base },
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
        d.get_hours(),
        d.get_minutes(),
        d.get_seconds()
    )
}

fn on_hotkey(ctx: Ctx, h: Hotkey) {
    match h {
        Hotkey::ToggleFullscreen => app::toggle_fullscreen(ctx),
        Hotkey::ToggleToolbar => {
            if ctx.fullscreen.get_untracked() {
                ctx.toolbar_shown.update(|s| *s = !*s);
            }
        }
    }
}

#[component]
pub fn Viewer(ctx: Ctx) -> impl IntoView {
    let prefs = ctx.prefs;
    let zoom = move || prefs.with(|p| p.zoom());
    let visible = move || {
        matches!(
            ctx.phase.get(),
            Phase::Connected | Phase::Reconnecting { .. }
        )
    };
    let hovered = RwSignal::new(false);

    // In full screen the toolbar slides in from the top edge.
    let reveal = move |ev: PointerEvent| {
        if ctx.fullscreen.get_untracked() && ev.client_y() <= 2 {
            ctx.toolbar_shown.set(true);
        }
    };
    let leave_toolbar = move |_| {
        hovered.set(false);
        set_timeout(
            move || {
                if !hovered.get_untracked() && ctx.menu.get_untracked().is_none() {
                    ctx.toolbar_shown.set(false);
                }
            },
            Duration::from_millis(600),
        );
    };

    let bell = RwSignal::new(false);
    Effect::new(move |prev: Option<u32>| {
        let n = ctx.bell.get();
        if prev.is_some_and(|p| p != n) {
            bell.set(true);
            set_timeout(move || bell.set(false), Duration::from_millis(350));
        }
        n
    });

    let size = move |side: fn(&crate::app::Desktop) -> u32| {
        let z = zoom();
        if z == FIT {
            String::new()
        } else {
            format!("{}px", ctx.desktop.with(side) * z as u32 / 100)
        }
    };

    view! {
        <div
            class="viewer"
            class:hidden=move || !visible()
            class:fullscreen=move || ctx.fullscreen.get()
            on:pointermove=reveal
        >
            <header
                class="toolbar"
                class:shown=move || ctx.toolbar_shown.get() || ctx.menu.get().is_some()
                on:mouseenter=move |_| hovered.set(true)
                on:mouseleave=leave_toolbar
            >
                <Status ctx />
                <div class="actions">
                    <Actions ctx />
                </div>
                <LangSwitch lang=ctx.lang />
            </header>
            <Show when=move || ctx.menu.get().is_some()>
                <div
                    class="menu-backdrop"
                    on:pointerdown=move |_| {
                        ctx.menu.set(None);
                        ctx.focus_canvas();
                    }
                ></div>
            </Show>
            <div
                class="screen"
                node_ref=ctx.screen
                class:fit=move || zoom() == FIT
                class:pixelated=move || !prefs.with(|p| p.smooth)
                class:bell=bell
                class:view-only=move || ctx.view_only.get()
            >
                <canvas
                    node_ref=ctx.canvas
                    tabindex="0"
                    width="0"
                    height="0"
                    style:width=move || size(|d| d.width)
                    style:height=move || size(|d| d.height)
                    on:pointerdown=move |ev: PointerEvent| {
                        ctx.menu.set(None);
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
                    on:keydown=move |ev: KeyboardEvent| {
                        if let Some(h) = viewer::on_key(&ev, true) {
                            on_hotkey(ctx, h)
                        }
                    }
                    on:keyup=move |ev: KeyboardEvent| {
                        viewer::on_key(&ev, false);
                    }
                    on:blur=move |_| viewer::release_all()
                    on:focus=move |_| tauri::invoke_detached("send_clipboard")
                ></canvas>
            </div>
            <Show when=move || matches!(ctx.phase.get(), Phase::Reconnecting { .. })>
                <ReconnectOverlay ctx />
            </Show>
        </div>
    }
}

/// Desktop name, size, security and throughput.
#[component]
fn Status(ctx: Ctx) -> impl IntoView {
    let t = move || ctx.t();
    let security = move || ctx.security.get();
    let encrypted = move || security().is_some_and(|s| s.encrypted);
    let security_title = move || match security() {
        Some(s) if s.tunnel => t().encrypted_ssh,
        Some(s) if s.encrypted => t().encrypted_tls,
        _ => t().not_encrypted,
    };
    view! {
        <div class="desktop-name">
            <span
                class="security"
                class:secure=encrypted
                title=security_title
                aria-label=security_title
                role="img"
            >
                {move || view! { <Svg icon=if encrypted() { Icon::Lock } else { Icon::Unlock } /> }}
            </span>
            <strong>{move || ctx.desktop.with(|d| d.name.clone())}</strong>
            <Show when=move || ctx.view_only.get()>
                <span class="badge">{move || t().view_only}</span>
            </Show>
            <span class="dims">
                {move || ctx.desktop.with(|d| format!("{} × {}", d.width, d.height))}
            </span>
            <span class="stats" title=move || t().stats_title>
                {move || {
                    let (frames, bytes) = ctx.stats.get();
                    let lang = ctx.lang.get();
                    format!("{frames} {} · {}", lang.texts().fps, format_rate(bytes, lang))
                }}
            </span>
        </div>
    }
}

#[component]
fn Actions(ctx: Ctx) -> impl IntoView {
    let t = move || ctx.t();
    let prefs = ctx.prefs;
    let toggle_menu = move |m: Menu| {
        ctx.menu
            .update(|cur| *cur = if *cur == Some(m) { None } else { Some(m) })
    };
    let then_focus = move || {
        ctx.menu.set(None);
        ctx.focus_canvas();
    };

    let paste = move |_| {
        tauri::invoke_detached("send_clipboard");
        then_focus();
    };
    let view_only = move |_| {
        let on = !ctx.view_only.get_untracked();
        viewer::set_view_only(on);
        ctx.view_only.set(on);
        then_focus();
    };
    let screenshot = move |_| {
        ctx.menu.set(None);
        let Some(canvas) = ctx.canvas.get_untracked() else {
            return;
        };
        let name = screenshot_name(&ctx.desktop.with_untracked(|d| d.name.clone()));
        spawn_local(async move {
            match viewer::screenshot(&canvas, &name).await {
                Ok(true) => {
                    ctx.show_toast(ctx.lang.get_untracked().texts().screenshot_saved.into())
                }
                Ok(false) => {}
                Err(e) => ctx.show_toast(e),
            }
            ctx.focus_canvas();
        });
    };
    let refresh = move |_| {
        tauri::invoke_detached("refresh");
        then_focus();
    };
    let fullscreen = move |_| {
        ctx.menu.set(None);
        app::toggle_fullscreen(ctx);
    };
    let new_window = move |_| {
        ctx.menu.set(None);
        tauri::invoke_detached("new_window");
    };

    let zoom_label = move || match prefs.with(|p| p.zoom()) {
        FIT => t().fit.to_owned(),
        z => format!("{z} %"),
    };

    view! {
        <div class="menu-anchor">
            <button
                class="icon-button"
                class:active=move || ctx.menu.get() == Some(Menu::Keys)
                title=move || t().keys_title
                aria-label=move || t().keys_title
                aria-haspopup="menu"
                aria-expanded=move || (ctx.menu.get() == Some(Menu::Keys)).to_string()
                on:click=move |_| toggle_menu(Menu::Keys)
            >
                <Svg icon=Icon::Keyboard />
                <span class="label">{move || t().keys}</span>
                <Svg icon=Icon::Chevron />
            </button>
            <Show when=move || ctx.menu.get() == Some(Menu::Keys)>
                <div class="menu" role="menu">
                    {move || {
                        combos(t())
                            .into_iter()
                            .map(|(label, keys)| {
                                view! {
                                    <button
                                        role="menuitem"
                                        on:click=move |_| {
                                            viewer::send_combo(keys);
                                            then_focus();
                                        }
                                    >
                                        {label}
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                    <div class="menu-title">{move || t().function_keys}</div>
                    <div class="fkeys">
                        {(1..=12u32)
                            .map(|n| {
                                view! {
                                    <button
                                        role="menuitem"
                                        on:click=move |_| {
                                            viewer::send_combo(&[0xffbd + n]);
                                            then_focus();
                                        }
                                    >
                                        {format!("F{n}")}
                                    </button>
                                }
                            })
                            .collect_view()}
                    </div>
                </div>
            </Show>
        </div>
        <button
            class="icon-button"
            title=move || t().clipboard_title
            aria-label=move || t().clipboard_title
            on:click=paste
        >
            <Svg icon=Icon::Clipboard />
        </button>
        <div class="menu-anchor">
            <button
                class="icon-button"
                class:active=move || ctx.menu.get() == Some(Menu::Zoom)
                title=move || t().zoom_title
                aria-label=move || t().zoom_title
                aria-haspopup="menu"
                aria-expanded=move || (ctx.menu.get() == Some(Menu::Zoom)).to_string()
                on:click=move |_| toggle_menu(Menu::Zoom)
            >
                <Svg icon=Icon::Zoom />
                <span class="label">{zoom_label}</span>
                <Svg icon=Icon::Chevron />
            </button>
            <Show when=move || ctx.menu.get() == Some(Menu::Zoom)>
                <div class="menu" role="menu">
                    {ZOOMS
                        .map(|z| {
                            let label = move || if z == FIT { t().fit_title.to_owned() } else { format!("{z} %") };
                            view! {
                                <button
                                    role="menuitemradio"
                                    aria-checked=move || prefs.with(|p| p.zoom() == z).to_string()
                                    class:checked=move || prefs.with(|p| p.zoom() == z)
                                    on:click=move |_| {
                                        prefs.update(|p| p.zoom = Some(z));
                                        then_focus();
                                    }
                                >
                                    {label}
                                </button>
                            }
                        })}
                    <hr />
                    <label class="check">
                        <input
                            type="checkbox"
                            prop:checked=move || prefs.with(|p| p.smooth)
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                prefs.update(|p| p.smooth = on);
                            }
                        />
                        <span>{move || t().smooth}</span>
                    </label>
                    <label class="check" class:disabled=move || !ctx.remote_resize_ok.get()>
                        <input
                            type="checkbox"
                            disabled=move || !ctx.remote_resize_ok.get()
                            prop:checked=move || prefs.with(|p| p.remote_resize)
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                prefs.update(|p| p.remote_resize = on);
                            }
                        />
                        <span>
                            {move || t().remote_resize}
                            <Show when=move || !ctx.remote_resize_ok.get()>
                                " "
                                <em>{move || t().remote_resize_unsupported}</em>
                            </Show>
                        </span>
                    </label>
                </div>
            </Show>
        </div>
        <button
            class="icon-button"
            class:active=move || ctx.view_only.get()
            title=move || t().view_only_title
            aria-label=move || t().view_only_title
            aria-pressed=move || ctx.view_only.get().to_string()
            on:click=view_only
        >
            {move || view! { <Svg icon=if ctx.view_only.get() { Icon::EyeOff } else { Icon::Eye } /> }}
        </button>
        <button
            class="icon-button"
            title=move || t().screenshot_title
            aria-label=move || t().screenshot_title
            on:click=screenshot
        >
            <Svg icon=Icon::Camera />
        </button>
        <button
            class="icon-button"
            title=move || t().refresh_title
            aria-label=move || t().refresh_title
            on:click=refresh
        >
            <Svg icon=Icon::Refresh />
        </button>
        <button
            class="icon-button"
            class:active=move || ctx.fullscreen.get()
            title=move || if ctx.fullscreen.get() { t().exit_fullscreen_title } else { t().fullscreen_title }
            aria-label=move || {
                if ctx.fullscreen.get() { t().exit_fullscreen_title } else { t().fullscreen_title }
            }
            on:click=fullscreen
        >
            {move || {
                view! { <Svg icon=if ctx.fullscreen.get() { Icon::Minimize } else { Icon::Maximize } /> }
            }}
        </button>
        <button
            class="icon-button"
            title=move || t().new_window_title
            aria-label=move || t().new_window_title
            on:click=new_window
        >
            <Svg icon=Icon::NewWindow />
        </button>
        <button
            class="icon-button danger"
            title=move || t().disconnect_title
            aria-label=move || t().disconnect_title
            on:click=move |_| app::disconnect(ctx)
        >
            <Svg icon=Icon::Power />
        </button>
    }
}

#[component]
fn ReconnectOverlay(ctx: Ctx) -> impl IntoView {
    let t = move || ctx.t();
    let status = move || match ctx.phase.get() {
        Phase::Reconnecting { attempt, at } => {
            let left = ((at - ctx.now.get()) / 1000.0).ceil();
            let line = if left > 0.0 {
                t().reconnecting_in.replace("{}", &left.to_string())
            } else {
                t().reconnecting.to_owned()
            };
            format!("{line} ({} {attempt})", t().attempt)
        }
        _ => String::new(),
    };
    view! {
        <div class="overlay">
            <div class="card dialog" role="alertdialog" aria-live="polite">
                <h2>{move || t().connection_lost}</h2>
                <p class="subtitle">
                    {move || ctx.error.get().map(|e| e.message(ctx.lang.get()))}
                </p>
                <p>{status}</p>
                <div class="buttons end">
                    <button type="button" on:click=move |_| app::cancel(ctx)>
                        {move || t().cancel}
                    </button>
                    <button type="button" class="primary" on:click=move |_| app::reconnect_now(ctx)>
                        {move || t().reconnect_now}
                    </button>
                </div>
            </div>
        </div>
    }
}
