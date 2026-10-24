//! Remote screen: draws backend messages on the canvas and forwards input.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use js_sys::{DataView, Reflect, Uint8Array, Uint8ClampedArray};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, ImageData, KeyboardEvent, MouseEvent,
    PointerEvent, WheelEvent,
};

use crate::i18n::UiError;
use crate::wire::{self, Bytes, ErrorInfo, Event, PixelBlock};
use crate::{keysym, tauri};

/// Wheel distance (in pixels) that makes one scroll "click".
const WHEEL_STEP: f64 = 50.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputEvent {
    Pointer { x: u16, y: u16, mask: u8 },
    Key { keysym: u32, down: bool },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub shared: bool,
    pub trusted_cert: Option<String>,
    pub quality: &'static str,
    pub ssh: Option<SshParams>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub trusted_key: Option<String>,
}

/// Shortcuts handled locally instead of being sent to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    /// Ctrl+Alt+Enter.
    ToggleFullscreen,
    /// F8.
    ToggleToolbar,
}

#[derive(Default)]
struct State {
    /// Incremented on every connection so that late messages of an old one are ignored.
    generation: u32,
    queue: Vec<InputEvent>,
    sending: bool,
    /// Keysym sent for each physically pressed key, so the release matches the press.
    pressed: HashMap<String, u32>,
    buttons: u8,
    pos: (u16, u16),
    wheel: f64,
    /// Input is not sent to the server.
    view_only: bool,
    /// Keys whose press was a [`Hotkey`]: their release is swallowed too.
    swallowed: HashSet<String>,
    /// Frames and bytes drawn since the last [`take_stats`].
    frames: u32,
    bytes: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

pub async fn connect(
    params: ConnectParams,
    canvas: HtmlCanvasElement,
    on_event: impl Fn(Event) + 'static,
) -> Result<(), UiError> {
    let generation = STATE.with_borrow_mut(|s| {
        s.generation += 1;
        s.generation
    });
    reset_input();

    let channel = tauri::Channel::new();
    let handler = Closure::<dyn FnMut(JsValue)>::new(move |data: JsValue| {
        if STATE.with_borrow(|s| s.generation) != generation {
            return;
        }
        if let Err(e) = handle_message(&data, &canvas, &on_event) {
            web_sys::console::error_2(&"invalid VNC message".into(), &e);
        }
    });
    channel.set_onmessage(handler.as_ref().unchecked_ref());
    // The channel lives as long as the page; so does its handler.
    handler.forget();

    let args = js_sys::Object::new();
    let params = params
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| UiError::Internal(e.to_string()))?;
    Reflect::set(&args, &"params".into(), &params)
        .map_err(|e| UiError::Internal(tauri::error_text(&e)))?;
    Reflect::set(&args, &"onMessage".into(), &channel)
        .map_err(|e| UiError::Internal(tauri::error_text(&e)))?;
    let args: JsValue = args.into();
    tauri::invoke_raw("connect", args)
        .await
        .map(|_| ())
        .map_err(
            |e| match serde_wasm_bindgen::from_value::<ErrorInfo>(e.clone()) {
                Ok(info) => UiError::Backend(info),
                Err(_) => UiError::Internal(tauri::error_text(&e)),
            },
        )
}

pub fn disconnect() {
    STATE.with_borrow_mut(|s| s.generation += 1);
    reset_input();
    tauri::invoke_detached("disconnect");
}

fn reset_input() {
    STATE.with_borrow_mut(|s| {
        s.queue.clear();
        s.pressed.clear();
        s.swallowed.clear();
        s.buttons = 0;
        s.wheel = 0.0;
    });
}

/// Frames drawn and bytes received since the last call.
pub fn take_stats() -> (u32, u64) {
    STATE.with_borrow_mut(|s| (std::mem::take(&mut s.frames), std::mem::take(&mut s.bytes)))
}

/// Stops (or resumes) sending keyboard and mouse input.
pub fn set_view_only(on: bool) {
    if on {
        release_all();
    }
    STATE.with_borrow_mut(|s| s.view_only = on);
}

/// Saves the canvas as PNG (the backend asks where). Returns whether it was saved.
pub async fn screenshot(canvas: &HtmlCanvasElement, file_name: &str) -> Result<bool, String> {
    let (tx, rx) = promise_channel();
    let cb = Closure::once(move |blob: JsValue| {
        let _ = tx.send(blob);
    });
    canvas
        .to_blob(cb.as_ref().unchecked_ref())
        .map_err(|e| tauri::error_text(&e))?;
    let blob: web_sys::Blob = rx
        .await
        .map_err(|_| "no image".to_string())?
        .dyn_into()
        .map_err(|_| "no image".to_string())?;
    let buf = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
        .await
        .map_err(|e| tauri::error_text(&e))?;
    let bytes = Uint8Array::new(&buf);
    let saved =
        tauri::invoke_bytes("save_screenshot", &bytes, &[("x-file-name", file_name)]).await?;
    Ok(saved.as_bool().unwrap_or(false))
}

/// A one-shot channel built on a JS promise.
fn promise_channel() -> (PromiseTx, wasm_bindgen_futures::JsFuture) {
    let mut resolve = None;
    let promise = js_sys::Promise::new(&mut |res, _rej| resolve = Some(res));
    (
        PromiseTx(resolve.expect("promise executor runs synchronously")),
        wasm_bindgen_futures::JsFuture::from(promise),
    )
}

struct PromiseTx(js_sys::Function);

impl PromiseTx {
    fn send(self, v: JsValue) -> Result<(), JsValue> {
        self.0.call1(&JsValue::NULL, &v).map(|_| ())
    }
}

// ---- Rendering --------------------------------------------------------------

struct JsBytes(DataView);

impl Bytes for JsBytes {
    fn len(&self) -> usize {
        self.0.byte_length()
    }
    fn u8_at(&self, i: usize) -> u8 {
        self.0.get_uint8(i)
    }
    fn u16_at(&self, i: usize) -> u16 {
        self.0.get_uint16_endian(i, true)
    }
    fn u32_at(&self, i: usize) -> u32 {
        self.0.get_uint32_endian(i, true)
    }
}

fn handle_message(
    data: &JsValue,
    canvas: &HtmlCanvasElement,
    on_event: &dyn Fn(Event),
) -> Result<(), JsValue> {
    // Raw channel payloads arrive as an ArrayBuffer; accept typed arrays too.
    let bytes = if data.is_instance_of::<Uint8Array>() {
        data.clone().unchecked_into()
    } else {
        Uint8Array::new(data)
    };
    if bytes.length() == 0 {
        return Ok(());
    }
    let view = JsBytes(DataView::new(
        &bytes.buffer(),
        bytes.byte_offset() as usize,
        bytes.byte_length() as usize,
    ));

    match view.u8_at(0) {
        wire::MSG_EVENT => {
            let json = bytes.subarray(1, bytes.length()).to_vec();
            let event: Event = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
            if let Event::Resize { width, height, .. } = &event {
                canvas.set_width(*width);
                canvas.set_height(*height);
            }
            on_event(event);
        }
        wire::MSG_RECTS => {
            STATE.with_borrow_mut(|s| {
                s.frames += 1;
                s.bytes += bytes.byte_length() as u64;
            });
            let ctx = context_2d(canvas)?;
            for block in wire::rects(&view)? {
                ctx.put_image_data(&image_data(&bytes, &block)?, block.x as f64, block.y as f64)?;
            }
            tauri::invoke_detached("frame_ack");
        }
        wire::MSG_CURSOR => set_cursor(canvas, &bytes, &wire::cursor(&view)?)?,
        k => return Err(format!("unknown message type {k}").into()),
    }
    Ok(())
}

fn context_2d(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, JsValue> {
    Ok(canvas
        .get_context("2d")?
        .ok_or("no 2d context")?
        .dyn_into()?)
}

/// ImageData viewing the message buffer directly (no copy through wasm).
fn image_data(bytes: &Uint8Array, block: &PixelBlock) -> Result<ImageData, JsValue> {
    let pixels = Uint8ClampedArray::new_with_byte_offset_and_length(
        &bytes.buffer(),
        bytes.byte_offset() + block.offset as u32,
        block.byte_len() as u32,
    );
    ImageData::new_with_js_u8_clamped_array_and_sh(&pixels, block.w, block.h)
}

fn set_cursor(
    canvas: &HtmlCanvasElement,
    bytes: &Uint8Array,
    c: &PixelBlock,
) -> Result<(), JsValue> {
    let style = canvas.style();
    if c.w == 0 || c.h == 0 {
        return style.set_property("cursor", "none");
    }
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let tmp: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
    tmp.set_width(c.w);
    tmp.set_height(c.h);
    context_2d(&tmp)?.put_image_data(&image_data(bytes, c)?, 0.0, 0.0)?;
    let url = tmp.to_data_url()?;
    style.set_property("cursor", &format!("url({url}) {} {}, default", c.x, c.y))
}

// ---- Input ------------------------------------------------------------------

/// Queues an input event; events are sent in order, batched while a call is in flight.
fn push(ev: InputEvent) {
    let dropped = STATE.with_borrow_mut(|s| {
        if s.view_only {
            return true;
        }
        // Successive moves with the same buttons: only the last position matters.
        if let (Some(InputEvent::Pointer { mask: last, .. }), InputEvent::Pointer { mask, .. }) =
            (s.queue.last(), ev)
            && *last == mask
        {
            s.queue.pop();
        }
        s.queue.push(ev);
        false
    });
    if !dropped {
        flush();
    }
}

fn flush() {
    let batch = STATE.with_borrow_mut(|s| {
        if s.sending || s.queue.is_empty() {
            return None;
        }
        s.sending = true;
        Some(std::mem::take(&mut s.queue))
    });
    let Some(events) = batch else { return };

    #[derive(Serialize)]
    struct Args {
        events: Vec<InputEvent>,
    }
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = tauri::invoke("input", &Args { events }).await {
            web_sys::console::warn_1(&format!("input: {e}").into());
        }
        STATE.with_borrow_mut(|s| s.sending = false);
        flush();
    });
}

/// Maps a mouse position to framebuffer coordinates. Handles `object-fit: contain`
/// scaling (letterboxing) as well as the 1:1 display.
fn position(ev: &MouseEvent, canvas: &HtmlCanvasElement) -> Option<(u16, u16)> {
    let (cw, ch) = (canvas.width() as f64, canvas.height() as f64);
    let r = canvas.get_bounding_client_rect();
    if cw == 0.0 || ch == 0.0 || r.width() == 0.0 || r.height() == 0.0 {
        return None;
    }
    let scale = (r.width() / cw).min(r.height() / ch);
    let left = r.left() + (r.width() - cw * scale) / 2.0;
    let top = r.top() + (r.height() - ch * scale) / 2.0;
    let x = ((ev.client_x() as f64 - left) / scale)
        .floor()
        .clamp(0.0, cw - 1.0);
    let y = ((ev.client_y() as f64 - top) / scale)
        .floor()
        .clamp(0.0, ch - 1.0);
    Some((x as u16, y as u16))
}

/// DOM `buttons` (1 left, 2 right, 4 middle) to the RFB mask (1 left, 2 middle, 4 right).
fn button_mask(buttons: u16) -> u8 {
    let mut mask = 0;
    if buttons & 1 != 0 {
        mask |= 1;
    }
    if buttons & 4 != 0 {
        mask |= 2;
    }
    if buttons & 2 != 0 {
        mask |= 4;
    }
    mask
}

pub fn on_pointer(ev: &PointerEvent, canvas: &HtmlCanvasElement) {
    let Some((x, y)) = position(ev, canvas) else {
        return;
    };
    let mask = button_mask(ev.buttons());
    STATE.with_borrow_mut(|s| {
        s.buttons = mask;
        s.pos = (x, y);
    });
    push(InputEvent::Pointer { x, y, mask });
}

pub fn on_pointer_down(ev: &PointerEvent, canvas: &HtmlCanvasElement) {
    ev.prevent_default();
    let _ = canvas.focus();
    let _ = canvas.set_pointer_capture(ev.pointer_id());
    on_pointer(ev, canvas);
}

pub fn on_wheel(ev: &WheelEvent, canvas: &HtmlCanvasElement) {
    ev.prevent_default();
    let Some((x, y)) = position(ev, canvas) else {
        return;
    };
    let dy = ev.delta_y();
    let steps = STATE.with_borrow_mut(|s| {
        if ev.delta_mode() == WheelEvent::DOM_DELTA_PIXEL {
            s.wheel += dy;
            let n = (s.wheel / WHEEL_STEP).trunc();
            s.wheel -= n * WHEEL_STEP;
            n as i32
        } else {
            dy.signum() as i32
        }
    });
    let base = STATE.with_borrow(|s| s.buttons);
    // Buttons 4 and 5 (mask bits 3 and 4) are wheel up and down.
    let bit = if steps < 0 { 8 } else { 16 };
    for _ in 0..steps.unsigned_abs().min(10) {
        push(InputEvent::Pointer {
            x,
            y,
            mask: base | bit,
        });
        push(InputEvent::Pointer { x, y, mask: base });
    }
}

fn hotkey(ev: &KeyboardEvent) -> Option<Hotkey> {
    let key = ev.key();
    if ev.ctrl_key() && ev.alt_key() && !ev.shift_key() && key == "Enter" {
        return Some(Hotkey::ToggleFullscreen);
    }
    if key == "F8" && !ev.ctrl_key() && !ev.alt_key() && !ev.shift_key() && !ev.meta_key() {
        return Some(Hotkey::ToggleToolbar);
    }
    None
}

/// Forwards a key event, unless it is a local shortcut: then it is returned
/// (on press) and nothing is sent, not even the release.
pub fn on_key(ev: &KeyboardEvent, down: bool) -> Option<Hotkey> {
    // Let the IME finish composing; the composed text arrives as key events after.
    if ev.is_composing() {
        return None;
    }
    ev.prevent_default();
    let (key, code) = (ev.key(), ev.code());
    let id = if code.is_empty() {
        key.clone()
    } else {
        code.clone()
    };
    if down && let Some(h) = hotkey(ev) {
        STATE.with_borrow_mut(|s| s.swallowed.insert(id));
        return (!ev.repeat()).then_some(h);
    }
    if !down && STATE.with_borrow_mut(|s| s.swallowed.remove(&id)) {
        return None;
    }
    let keysym = STATE.with_borrow_mut(|s| {
        if down {
            let ks = keysym::keysym(&key, &code)?;
            s.pressed.insert(id, ks);
            Some(ks)
        } else {
            s.pressed
                .remove(&id)
                .or_else(|| keysym::keysym(&key, &code))
        }
    });
    if let Some(keysym) = keysym {
        push(InputEvent::Key { keysym, down });
    }
    None
}

/// Releases every key and button still held (e.g. when the canvas loses focus).
pub fn release_all() {
    let (keys, buttons, (x, y)) = STATE.with_borrow_mut(|s| {
        let keys: Vec<u32> = s.pressed.drain().map(|(_, k)| k).collect();
        (keys, std::mem::take(&mut s.buttons), s.pos)
    });
    for keysym in keys {
        push(InputEvent::Key {
            keysym,
            down: false,
        });
    }
    if buttons != 0 {
        push(InputEvent::Pointer { x, y, mask: 0 });
    }
}

/// Presses the keys in order, then releases them in reverse order.
pub fn send_combo(keysyms: &[u32]) {
    for &keysym in keysyms {
        push(InputEvent::Key { keysym, down: true });
    }
    for &keysym in keysyms.iter().rev() {
        push(InputEvent::Key {
            keysym,
            down: false,
        });
    }
}
