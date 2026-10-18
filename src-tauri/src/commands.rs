//! Tauri commands for chat/system history, the translator toggle and furigana. A command
//! the bridge also runs has a plain function beside it, which both call.

use crate::{inject_system_message, AppState, ChatMessage, SystemLogLevel, SystemMessage};
use resonance_types::RubySpan;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn clear_chat_history(app: AppHandle) {
    clear_history(&app);
}

/// Empties the backend's chat log and system log (the clear button).
pub fn clear_history(app: &AppHandle) {
    let state = app.state::<AppState>();
    // 1. Clear Game Chat
    state.chat_history.lock().clear();

    // 2. Clear System Logs (Optional, but good for a full reset)
    state.system_history.lock().clear();
}

#[tauri::command]
pub fn ui_system_message(
    app: tauri::AppHandle, // Tauri prefers AppHandle passed by value in commands
    level: String,
    source: String,  // Use concrete String for frontend IPC
    message: String, // Use concrete String for frontend IPC
) {
    inject_system_message(&app, SystemLogLevel::parse(&level), &source, message);
}

#[tauri::command]
pub fn get_chat_history(app: AppHandle) -> Vec<ChatMessage> {
    chat_history(&app)
}

/// The backend's chat log, in arrival order: game chat only, no system lines.
pub fn chat_history(app: &AppHandle) -> Vec<ChatMessage> {
    let state = app.state::<AppState>();
    let history = state.chat_history.lock();
    history.values().cloned().collect()
}

#[tauri::command]
pub fn get_system_history(state: tauri::State<AppState>) -> Vec<SystemMessage> {
    // Change: Returns specialized SystemMessages
    let history = state.system_history.lock();
    history.iter().cloned().collect()
}

/// The last state each service reported. The UI asks once its listeners are
/// registered: states emitted before that (the translator starts with the
/// app) would otherwise be lost.
#[tauri::command]
pub fn get_service_states(state: tauri::State<AppState>) -> crate::ServiceStates {
    state.service_states.lock().clone()
}

#[tauri::command]
pub fn launch_translator(app: AppHandle, state: State<'_, AppState>) {
    // Idempotent: a translator that is already running is left alone.
    state.services.start_translator(&app);
}

/// async so that it runs on the async runtime, not the main thread.
#[tauri::command]
pub async fn annotate_furigana(texts: Vec<String>) -> Vec<Vec<RubySpan>> {
    furigana(&texts)
}

/// Furigana for Japanese lines, one list of spans per line, in order. The
/// analysis itself is `resonance_core::furigana`; it takes microseconds a line.
pub fn furigana(texts: &[String]) -> Vec<Vec<RubySpan>> {
    texts
        .iter()
        .map(|text| resonance_core::furigana::annotate(text))
        .collect()
}
