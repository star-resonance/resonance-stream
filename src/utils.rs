use crate::tauri_bridge::call;
use crate::ui_types::SystemLogLevel;
use leptos::task::spawn_local;
use resonance_types::ipc::UiSystemMessage;
use wasm_bindgen::prelude::*;

/// A window-wide listener that ends with the component that made it. Leptos's own
/// `window_event_listener` does not: it stays on the window after the component is gone, and the
/// next event reaches its closure, which reads signals that were disposed with the component and
/// panics (the title bar's `mousedown` did that on every click once compact mode removed the bar).
/// Call it where `window_event_listener` was called, inside a component.
pub fn window_listener<E>(event: E, cb: impl Fn(E::EventType) + 'static)
where
    E: leptos::ev::EventDescriptor + 'static,
    E::EventType: JsCast,
{
    let handle = leptos::prelude::window_event_listener(event, cb);
    leptos::prelude::on_cleanup(move || handle.remove());
}

pub fn format_time(ts: u64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(ts as f64 * 1000.0));
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

/// The backend's rule (`resonance_types::contains_japanese`), so the UI
/// marks exactly the lines the translator takes.
pub fn is_japanese(text: &str) -> bool {
    crate::ui_types::contains_japanese(text)
}

pub fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.navigator().clipboard().write_text(text);
    }
}

pub fn add_system_log(level: SystemLogLevel, source: &str, message: &str) {
    let line = UiSystemMessage {
        level: level.as_str().to_string(),
        source: source.to_string(),
        message: message.to_string(),
    };

    spawn_local(async move {
        // This triggers the backend which emits 'system-event'
        // that your existing listener already handles
        let _ = call(&line).await;
    });
}
