//! Connection screen: form, history, errors and credential prompts.

use leptos::ev::SubmitEvent;
use leptos::html;
use leptos::prelude::*;

use crate::app::{self, Ctx, Phase, Prompt};
use crate::i18n::{Lang, UiError};
use crate::icons::{Icon, Svg};
use crate::prefs::{Quality, Server};
use crate::{address, secrets};

/// FR / EN toggle. Doesn't take the focus, so the remote screen keeps the keyboard.
#[component]
pub fn LangSwitch(lang: RwSignal<Lang>) -> impl IntoView {
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
pub fn ConnectScreen(ctx: Ctx) -> impl IntoView {
    let t = move || ctx.t();
    let connecting = move || ctx.phase.get() == Phase::Connecting;
    let prefs = ctx.prefs;

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if connecting() {
            return;
        }
        match ctx.port.get_untracked().trim().parse::<u16>() {
            Ok(p) if p > 0 => prefs.update(|prefs| prefs.port = p),
            _ => return ctx.error.set(Some(UiError::InvalidPort)),
        }
        app::start(ctx, ctx.form_target());
    };

    view! {
        <div class="connect-screen">
            // The form is out of reach while a dialog is open.
            <div class="connect-column" inert=move || ctx.prompt.get().is_some()>
                <form class="card" on:submit=submit>
                    <LangSwitch lang=ctx.lang />
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
                            autocomplete="off"
                            prop:value=ctx.host
                            on:input=move |ev| ctx.host.set(event_target_value(&ev))
                        />
                    </label>

                    <details class="advanced">
                        <summary>
                            {move || t().advanced}
                            // The tunnel applies even when the options are folded.
                            {move || {
                                let ssh = ctx.ssh.get();
                                (!ssh.trim().is_empty())
                                    .then(|| {
                                        view! {
                                            <span class="summary-note">
                                                {format!(" · {} {} {}", t().ssh_tunnel, t().via, ssh.trim())}
                                            </span>
                                        }
                                    })
                            }}
                        </summary>
                        <div class="advanced-body">
                            <label>
                                <span>{move || t().ssh_tunnel}</span>
                                <input
                                    type="text"
                                    placeholder=move || t().ssh_placeholder
                                    spellcheck="false"
                                    autocomplete="off"
                                    prop:value=ctx.ssh
                                    on:input=move |ev| ctx.ssh.set(event_target_value(&ev))
                                />
                            </label>
                            <div class="row">
                                <label>
                                    <span>{move || t().username}</span>
                                    <input
                                        type="text"
                                        spellcheck="false"
                                        autocomplete="off"
                                        prop:value=ctx.username
                                        on:input=move |ev| {
                                            ctx.username.set(event_target_value(&ev))
                                        }
                                    />
                                </label>
                                <label class="narrow">
                                    <span>{move || t().port}</span>
                                    <input
                                        type="number"
                                        min="1"
                                        max="65535"
                                        prop:value=ctx.port
                                        on:input=move |ev| ctx.port.set(event_target_value(&ev))
                                    />
                                </label>
                            </div>
                            <label>
                                <span>{move || t().quality}</span>
                                <select on:change=move |ev| {
                                    let q = Quality::from_key(&event_target_value(&ev));
                                    prefs.update(|p| p.quality = q);
                                }>
                                    {Quality::ALL
                                        .map(|q| {
                                            let label = move || match q {
                                                Quality::Lossless => t().quality_lossless,
                                                Quality::High => t().quality_high,
                                                Quality::Low => t().quality_low,
                                            };
                                            view! {
                                                <option
                                                    value=q.key()
                                                    selected=move || prefs.with(|p| p.quality == q)
                                                >
                                                    {label}
                                                </option>
                                            }
                                        })}
                                </select>
                            </label>
                            <label class="check">
                                <input
                                    type="checkbox"
                                    prop:checked=move || prefs.with(|p| p.shared)
                                    on:change=move |ev| {
                                        let on = event_target_checked(&ev);
                                        prefs.update(|p| p.shared = on);
                                    }
                                />
                                <span>{move || t().shared}</span>
                            </label>
                            <label class="check">
                                <input
                                    type="checkbox"
                                    prop:checked=move || prefs.with(|p| p.auto_reconnect)
                                    on:change=move |ev| {
                                        let on = event_target_checked(&ev);
                                        prefs.update(|p| p.auto_reconnect = on);
                                    }
                                />
                                <span>{move || t().auto_reconnect}</span>
                            </label>
                        </div>
                    </details>

                    <Show when=move || ctx.error.get().is_some() && ctx.prompt.get().is_none()>
                        <ErrorPanel ctx />
                    </Show>

                    <div class="buttons">
                        <button type="submit" class="primary" disabled=connecting>
                            <Show when=connecting>
                                <span class="spinner" aria-hidden="true"></span>
                            </Show>
                            {move || if connecting() { t().connecting } else { t().connect }}
                        </button>
                        <Show when=connecting>
                            <button type="button" on:click=move |_| app::cancel(ctx)>
                                {move || t().cancel}
                            </button>
                        </Show>
                    </div>
                </form>

                <History ctx />
                <p class="version">"VNC Viewer " {env!("CARGO_PKG_VERSION")}</p>
            </div>
            <Show when=move || ctx.prompt.get().is_some()>
                <CredentialsDialog ctx />
            </Show>
        </div>
    }
}

/// The error, with the actions that can fix it.
#[component]
fn ErrorPanel(ctx: Ctx) -> impl IntoView {
    let t = move || ctx.t();
    let error = move || ctx.error.get();
    let code = move || error().and_then(|e| e.code().map(str::to_owned));
    let detail = move || match error() {
        Some(UiError::Backend(info)) => info.detail.unwrap_or_default(),
        _ => String::new(),
    };

    let trust_cert = move |_| {
        let Some(target) = ctx.current.get_value() else {
            return;
        };
        let port = ctx.prefs.with_untracked(|p| p.port);
        if let Ok((host, port)) = address::resolve(&target.server.address, port) {
            let fp = detail();
            ctx.prefs.update(|p| {
                p.trusted_certs.insert(format!("{host}:{port}"), fp);
            });
            app::start(ctx, target);
        }
    };
    let trust_ssh = move |_| {
        if let Some(mut target) = ctx.current.get_value() {
            target.trusted_ssh_key = Some(detail());
            app::start(ctx, target);
        }
    };
    let retry = move |_| {
        if let Some(target) = ctx.current.get_value() {
            app::start(ctx, target);
        }
    };
    let can_retry = move || {
        ctx.current.with_value(Option::is_some)
            && error().is_some_and(|e| matches!(e, UiError::SessionEnded) || e.is_transient())
    };

    view! {
        <div class="error" role="alert">
            <p class="message">{move || error().map(|e| e.message(ctx.lang.get()))}</p>
            {move || match code().as_deref() {
                Some("untrustedCertificate") => {
                    view! {
                        <p>{t().untrusted_cert}</p>
                        <code class="fingerprint">{detail()}</code>
                        <button type="button" on:click=trust_cert>
                            {t().trust_and_connect}
                        </button>
                    }
                        .into_any()
                }
                Some("sshHostKeyUnknown") => {
                    view! {
                        <p>{t().ssh_unknown_key}</p>
                        <code class="fingerprint">{detail()}</code>
                        <button type="button" on:click=trust_ssh>
                            {t().trust_and_connect}
                        </button>
                    }
                        .into_any()
                }
                _ if can_retry() => {
                    view! {
                        <button type="button" on:click=retry>
                            {t().retry}
                        </button>
                    }
                        .into_any()
                }
                _ => ().into_any(),
            }}
        </div>
    }
}

#[component]
fn History(ctx: Ctx) -> impl IntoView {
    let t = move || ctx.t();
    let servers = move || ctx.prefs.with(|p| p.sorted_servers());

    let open = move |s: Server| {
        ctx.fill_form(&s);
        app::start(
            ctx,
            app::Target {
                server: s,
                ..Default::default()
            },
        );
    };
    let toggle_favorite = move |s: Server| {
        ctx.prefs.update(|p| {
            if let Some(e) = p.servers.iter_mut().find(|e| e.same_target(&s)) {
                e.favorite = !e.favorite;
            }
        })
    };
    let forget = move |s: Server| {
        if s.remember {
            secrets::delete(secrets::vnc_key(&s.address, &s.username, &s.ssh));
            if !s.ssh.is_empty() {
                secrets::delete(secrets::ssh_key(&s.ssh));
            }
        }
        ctx.prefs
            .update(|p| p.servers.retain(|e| !e.same_target(&s)));
    };

    view! {
        <Show when=move || !servers().is_empty()>
            <section class="card history">
                <h2>{move || t().recent}</h2>
                <ul>
                    {move || {
                        servers()
                            .into_iter()
                            .map(|s| {
                                let title = if s.name.is_empty() {
                                    s.address.clone()
                                } else {
                                    s.name.clone()
                                };
                                let mut sub = s.address.clone();
                                if !s.username.is_empty() {
                                    sub = format!("{}@{}", s.username, sub);
                                }
                                let via = (!s.ssh.is_empty()).then(|| s.ssh.clone());
                                let (s1, s2, s3) = (s.clone(), s.clone(), s.clone());
                                let fav = s.favorite;
                                view! {
                                    <li>
                                        <button
                                            type="button"
                                            class="star"
                                            class:on=fav
                                            title=move || {
                                                if fav { t().favorite_remove } else { t().favorite_add }
                                            }
                                            aria-label=move || {
                                                if fav { t().favorite_remove } else { t().favorite_add }
                                            }
                                            aria-pressed=fav.to_string()
                                            on:click=move |_| toggle_favorite(s1.clone())
                                        >
                                            <Svg icon=Icon::Star />
                                        </button>
                                        <button
                                            type="button"
                                            class="entry"
                                            on:click=move |_| open(s2.clone())
                                        >
                                            <strong>{title}</strong>
                                            <span class="sub">
                                                {sub}
                                                {via
                                                    .map(|v| {
                                                        view! {
                                                            " · "
                                                            {move || t().via}
                                                            " "
                                                            {v}
                                                        }
                                                    })}
                                            </span>
                                        </button>
                                        <Show when=move || s.remember>
                                            <span class="remembered" title=move || t().remember_password>
                                                <Svg icon=Icon::Key />
                                            </span>
                                        </Show>
                                        <button
                                            type="button"
                                            class="forget"
                                            title=move || t().forget
                                            aria-label=move || t().forget
                                            on:click=move |_| forget(s3.clone())
                                        >
                                            <Svg icon=Icon::Close />
                                        </button>
                                    </li>
                                }
                            })
                            .collect_view()
                    }}
                </ul>
            </section>
        </Show>
    }
}

/// Asks for the password (and user name) the server or SSH gateway wants.
#[component]
fn CredentialsDialog(ctx: Ctx) -> impl IntoView {
    let t = move || ctx.t();
    let prompt = ctx.prompt.get_untracked().expect("shown with a prompt");
    let target = ctx.current.get_value().unwrap_or_default();
    let ssh = matches!(prompt, Prompt::Ssh { .. });
    let need_user = matches!(
        prompt,
        Prompt::Vnc {
            need_user: true,
            ..
        }
    );
    let failed = matches!(
        prompt,
        Prompt::Vnc { failed: true, .. } | Prompt::Ssh { failed: true }
    );

    let username = RwSignal::new(target.server.username.clone());
    let password = RwSignal::new(String::new());
    let show = RwSignal::new(false);
    let remember = RwSignal::new(target.server.remember);
    let ssh_target = target.server.ssh.clone();
    // `autofocus` is ignored for inserted content: focus the first empty field.
    let user_ref = NodeRef::<html::Input>::new();
    let password_ref = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        let field = match (user_ref.get(), password_ref.get()) {
            (Some(u), _) if u.value().is_empty() => Some(u),
            (_, p) => p,
        };
        if let Some(f) = field {
            let _ = f.focus();
        }
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Some(mut target) = ctx.current.get_value() else {
            return;
        };
        if need_user {
            target.server.username = username.get_untracked().trim().to_owned();
            ctx.username.set(target.server.username.clone());
        }
        if ssh {
            target.ssh_password = Some(password.get_untracked());
        } else {
            target.password = Some(password.get_untracked());
        }
        target.server.remember = remember.get_untracked();
        if !target.server.remember {
            secrets::delete(if ssh {
                target.ssh_key()
            } else {
                target.vnc_key()
            });
        }
        app::start(ctx, target);
    };
    let close = move || {
        ctx.prompt.set(None);
        ctx.error.set(None);
    };

    view! {
        <div
            class="modal-backdrop"
            on:keydown=move |ev| {
                if ev.key() == "Escape" {
                    close();
                }
            }
        >
            <form class="card dialog" role="dialog" aria-modal="true" on:submit=submit>
                <h2>{move || if ssh { t().ssh_auth_title } else { t().auth_title }}</h2>
                <p class="subtitle">
                    {move || {
                        if ssh {
                            format!("{} {}", t().ssh_auth_hint, ssh_target)
                        } else if need_user {
                            t().auth_credentials_hint.to_owned()
                        } else {
                            t().auth_password_hint.to_owned()
                        }
                    }}
                </p>
                <Show when=move || failed>
                    <p class="error" role="alert">
                        {move || match ctx.error.get() {
                            // The server's own reason (e.g. too many attempts).
                            Some(UiError::Backend(info)) if info.code == "authFailed" => {
                                info.detail.unwrap_or_else(|| t().auth_failed_hint.to_owned())
                            }
                            _ => t().auth_failed_hint.to_owned(),
                        }}
                    </p>
                </Show>
                <Show when=move || need_user>
                    <label>
                        <span>{move || t().username}</span>
                        <input
                            node_ref=user_ref
                            type="text"
                            spellcheck="false"
                            autocomplete="username"
                            prop:value=username
                            on:input=move |ev| username.set(event_target_value(&ev))
                        />
                    </label>
                </Show>
                <label>
                    <span>{move || t().password}</span>
                    <div class="password">
                        <input
                            node_ref=password_ref
                            type=move || if show.get() { "text" } else { "password" }
                            autocomplete="current-password"
                            prop:value=password
                            on:input=move |ev| password.set(event_target_value(&ev))
                        />
                        <button
                            type="button"
                            class="icon-button"
                            title=move || if show.get() { t().hide_password } else { t().show_password }
                            aria-label=move || {
                                if show.get() { t().hide_password } else { t().show_password }
                            }
                            aria-pressed=move || show.get().to_string()
                            on:click=move |_| show.update(|s| *s = !*s)
                        >
                            {move || {
                                view! { <Svg icon=if show.get() { Icon::EyeOff } else { Icon::Eye } /> }
                            }}
                        </button>
                    </div>
                </label>
                <label class="check">
                    <input
                        type="checkbox"
                        prop:checked=remember
                        on:change=move |ev| remember.set(event_target_checked(&ev))
                    />
                    <span>{move || t().remember_password}</span>
                </label>
                <div class="buttons end">
                    <button type="button" on:click=move |_| close()>
                        {move || t().cancel}
                    </button>
                    <button type="submit" class="primary">
                        {move || t().connect}
                    </button>
                </div>
            </form>
        </div>
    }
}
