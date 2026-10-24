mod clipboard;
mod rfb;
mod session;
mod ssh;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::ipc::{Channel, InvokeBody, InvokeResponseBody, Request};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_window_state::StateFlags;
use tokio::sync::Notify;

use clipboard::ClipboardSync;
use session::{ConnectParams, InputEvent, Session, Sink};

/// Service name of the passwords saved in the system keychain.
const KEYRING_SERVICE: &str = "io.github.vncviewer";

/// The session of one window.
#[derive(Default)]
struct Slot {
    session: Option<Session>,
    /// Incremented by every connect and disconnect: a connection that
    /// completes after a newer request is dropped.
    attempt: u64,
    /// Wakes the pending connection attempt to cancel it.
    cancel: Arc<Notify>,
}

#[derive(Default)]
struct AppState {
    /// Sessions by window label.
    slots: Mutex<HashMap<String, Slot>>,
    clipboard: Arc<ClipboardSync>,
    windows: AtomicU32,
}

impl AppState {
    fn with_session(&self, window: &WebviewWindow, f: impl FnOnce(&Session)) {
        if let Some(s) = self
            .slots
            .lock()
            .unwrap()
            .get(window.label())
            .and_then(|slot| slot.session.as_ref())
        {
            f(s)
        }
    }
}

struct ChannelSink {
    channel: Channel<InvokeResponseBody>,
    clipboard: Arc<ClipboardSync>,
}

impl Sink for ChannelSink {
    fn message(&self, bytes: Vec<u8>) {
        if let Err(e) = self.channel.send(InvokeResponseBody::Raw(bytes)) {
            log::warn!("cannot send to the UI: {e}");
        }
    }

    fn cut_text(&self, text: String) {
        let clipboard = self.clipboard.clone();
        tauri::async_runtime::spawn_blocking(move || clipboard.set_from_server(text));
    }
}

#[tauri::command]
async fn connect(
    window: WebviewWindow,
    state: State<'_, AppState>,
    params: ConnectParams,
    on_message: Channel<InvokeResponseBody>,
) -> Result<(), rfb::ErrorInfo> {
    let label = window.label().to_owned();
    let (attempt, cancel) = {
        let mut slots = state.slots.lock().unwrap();
        let slot = slots.entry(label.clone()).or_default();
        // Close any previous session first.
        slot.session.take();
        slot.attempt += 1;
        slot.cancel = Arc::new(Notify::new());
        (slot.attempt, slot.cancel.clone())
    };
    let sink = ChannelSink {
        channel: on_message,
        clipboard: state.clipboard.clone(),
    };
    let session = tokio::select! {
        r = Session::connect(&params, sink) => r,
        _ = cancel.notified() => Err(rfb::Error::Cancelled),
    }
    .map_err(|e| {
        log::warn!("connection failed: {e}");
        e.info()
    })?;
    let mut slots = state.slots.lock().unwrap();
    let slot = slots.entry(label).or_default();
    if slot.attempt != attempt {
        return Err(rfb::Error::Cancelled.info());
    }
    slot.session = Some(session);
    Ok(())
}

/// Closes the session, or cancels the connection in progress.
#[tauri::command]
fn disconnect(window: WebviewWindow, state: State<'_, AppState>) {
    if let Some(slot) = state.slots.lock().unwrap().get_mut(window.label()) {
        slot.session.take();
        slot.attempt += 1;
        slot.cancel.notify_one();
    }
}

#[tauri::command]
fn input(window: WebviewWindow, state: State<'_, AppState>, events: Vec<InputEvent>) {
    state.with_session(&window, |s| s.input(&events));
}

#[tauri::command]
fn frame_ack(window: WebviewWindow, state: State<'_, AppState>) {
    state.with_session(&window, Session::frame_ack);
}

#[tauri::command]
fn refresh(window: WebviewWindow, state: State<'_, AppState>) {
    state.with_session(&window, Session::refresh);
}

#[tauri::command]
fn set_desktop_size(window: WebviewWindow, state: State<'_, AppState>, width: u16, height: u16) {
    state.with_session(&window, |s| s.set_desktop_size(width, height));
}

/// Sends the local clipboard to the server if it changed. Returns whether it was sent.
#[tauri::command]
async fn send_clipboard(window: WebviewWindow, state: State<'_, AppState>) -> Result<bool, String> {
    let clipboard = state.clipboard.clone();
    let text = tauri::async_runtime::spawn_blocking(move || clipboard.take_local_if_changed())
        .await
        .map_err(|e| e.to_string())?;
    let mut sent = false;
    if let Some(text) = text {
        state.with_session(&window, |s| {
            s.send_cut_text(&text);
            sent = true;
        });
    }
    Ok(sent)
}

#[tauri::command]
fn toggle_fullscreen(window: WebviewWindow) -> Result<bool, String> {
    let fullscreen = !window.is_fullscreen().map_err(|e| e.to_string())?;
    window
        .set_fullscreen(fullscreen)
        .map_err(|e| e.to_string())?;
    Ok(fullscreen)
}

#[tauri::command]
fn set_title(window: WebviewWindow, title: String) -> Result<(), String> {
    window.set_title(&title).map_err(|e| e.to_string())
}

/// Opens another viewer window, with its own session.
#[tauri::command]
async fn new_window(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let n = state.windows.fetch_add(1, Ordering::Relaxed) + 1;
    WebviewWindowBuilder::new(&app, format!("vnc-{n}"), WebviewUrl::default())
        .title("VNC Viewer")
        .inner_size(1280.0, 820.0)
        .min_inner_size(480.0, 360.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Asks where to save the PNG sent as the request body. Returns whether it was saved.
#[tauri::command]
async fn save_screenshot(app: AppHandle, request: Request<'_>) -> Result<bool, String> {
    let InvokeBody::Raw(png) = request.body() else {
        return Err("expected PNG bytes".into());
    };
    let png = png.clone();
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("screenshot.png")
        .to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("PNG", &["png"])
        .set_file_name(name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    tokio::fs::write(&path, png)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

fn keyring_entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| e.to_string())
}

/// Secret saved in the system keychain under `key`, if any.
#[tauri::command]
async fn secret_get(key: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || match keyring_entry(&key)?.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn secret_set(key: String, secret: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        keyring_entry(&key)?
            .set_password(&secret)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn secret_delete(key: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || match keyring_entry(&key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .manage(AppState::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                state.slots.lock().unwrap().remove(window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            input,
            frame_ack,
            refresh,
            set_desktop_size,
            send_clipboard,
            toggle_fullscreen,
            set_title,
            new_window,
            save_screenshot,
            secret_get,
            secret_set,
            secret_delete
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
