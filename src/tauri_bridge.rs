use resonance_types::ipc::Command;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

#[wasm_bindgen]
extern "C" {
    // Private: a command is called through `call` and its `resonance_types::ipc` struct, never by
    // a name typed at the call site.
    #[wasm_bindgen(catch, js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    pub async fn listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> JsValue;
}

/// What a failed call says: the backend's error text, or the value itself when it is not text.
pub fn js_reason(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

/// Calls the backend command `C` with `args` (`resonance_types::ipc`) and decodes its reply. The
/// error is the backend's message, or says which command's arguments or reply could not be read.
pub async fn call<C: Command>(args: &C) -> Result<C::Reply, String> {
    let args = serde_wasm_bindgen::to_value(args)
        .map_err(|e| format!("{}: the arguments did not encode: {e}", C::NAME))?;
    let reply = invoke(C::NAME, args).await.map_err(js_reason)?;
    serde_wasm_bindgen::from_value(reply)
        .map_err(|e| format!("{}: the reply did not decode: {e}", C::NAME))
}
