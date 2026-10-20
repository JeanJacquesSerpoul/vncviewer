//! Bindings to the Tauri JavaScript API (`withGlobalTauri`).

use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

#[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
extern "C" {
    /// `window.__TAURI__.core.Channel`: streams messages from a command.
    pub type Channel;

    #[wasm_bindgen(constructor)]
    pub fn new() -> Channel;

    #[wasm_bindgen(method, setter)]
    pub fn set_onmessage(this: &Channel, handler: &js_sys::Function);
}

/// Calls a command; the error is the raw value the command rejected with.
pub async fn invoke_raw(cmd: &str, args: JsValue) -> Result<JsValue, JsValue> {
    tauri_invoke(cmd, args).await
}

pub fn error_text(e: &JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

pub async fn invoke_value(cmd: &str, args: JsValue) -> Result<JsValue, String> {
    invoke_raw(cmd, args).await.map_err(|e| error_text(&e))
}

pub async fn invoke<A: Serialize>(cmd: &str, args: &A) -> Result<JsValue, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    invoke_value(cmd, args).await
}

/// Fire-and-forget call of a command without arguments.
pub fn invoke_detached(cmd: &'static str) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = invoke_value(cmd, JsValue::UNDEFINED).await {
            web_sys::console::warn_1(&format!("{cmd} : {e}").into());
        }
    });
}
