mod clipboard;
mod rfb;
mod session;

use std::sync::{Arc, Mutex};

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};

use clipboard::ClipboardSync;
use session::{InputEvent, Session, Sink};

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
    clipboard: Arc<ClipboardSync>,
}

impl AppState {
    fn with_session(&self, f: impl FnOnce(&Session)) {
        if let Some(s) = self.session.lock().unwrap().as_ref() {
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
    state: State<'_, AppState>,
    host: String,
    port: u16,
    password: Option<String>,
    shared: bool,
    on_message: Channel<InvokeResponseBody>,
) -> Result<(), rfb::ErrorInfo> {
    // Close any previous session first.
    state.session.lock().unwrap().take();
    let sink = ChannelSink {
        channel: on_message,
        clipboard: state.clipboard.clone(),
    };
    let password = password.filter(|p| !p.is_empty());
    let session = Session::connect(host.trim(), port, password.as_deref(), shared, sink)
        .await
        .map_err(|e| {
            log::warn!("connection failed: {e}");
            e.info()
        })?;
    *state.session.lock().unwrap() = Some(session);
    Ok(())
}

#[tauri::command]
fn disconnect(state: State<'_, AppState>) {
    state.session.lock().unwrap().take();
}

#[tauri::command]
fn input(state: State<'_, AppState>, events: Vec<InputEvent>) {
    state.with_session(|s| s.input(&events));
}

#[tauri::command]
fn frame_ack(state: State<'_, AppState>) {
    state.with_session(Session::frame_ack);
}

#[tauri::command]
fn refresh(state: State<'_, AppState>) {
    state.with_session(Session::refresh);
}

/// Sends the local clipboard to the server if it changed. Returns whether it was sent.
#[tauri::command]
async fn send_clipboard(state: State<'_, AppState>) -> Result<bool, String> {
    let clipboard = state.clipboard.clone();
    let text = tauri::async_runtime::spawn_blocking(move || clipboard.take_local_if_changed())
        .await
        .map_err(|e| e.to_string())?;
    let mut sent = false;
    if let Some(text) = text {
        state.with_session(|s| {
            s.send_cut_text(&text);
            sent = true;
        });
    }
    Ok(sent)
}

#[tauri::command]
fn toggle_fullscreen(window: tauri::WebviewWindow) -> Result<bool, String> {
    let fullscreen = !window.is_fullscreen().map_err(|e| e.to_string())?;
    window
        .set_fullscreen(fullscreen)
        .map_err(|e| e.to_string())?;
    Ok(fullscreen)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            input,
            frame_ack,
            refresh,
            send_clipboard,
            toggle_fullscreen
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
