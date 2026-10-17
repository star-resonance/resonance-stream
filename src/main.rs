mod app;
pub mod chat_view;
pub mod cheatsheet;
pub mod components;
pub mod config_signals;
pub mod dictionary_edit;
pub mod dictionary_status;
pub mod download_progress;
pub mod favorites;
pub mod favorites_sync;
pub mod hooks;
pub mod popup_view;
pub mod readability;
pub mod ruby_view;
pub mod service_state;
pub mod settings_nav;
pub mod setup_plan;
pub mod shortcut_keys;
pub mod status_signals;
pub mod status_view;
pub mod store;
pub mod tauri_bridge;
pub mod translation_view;
pub mod ui_types;
pub mod utils;
pub mod view_signals;

use app::*;
use components::PopupApp;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    match popup_view::current_popup() {
        // A popup window (open_popup): its tool alone, no chat.
        Some(kind) => mount_to_body(move || view! { <PopupApp kind=kind/> }),
        None => mount_to_body(|| view! { <App/> }),
    }
}
