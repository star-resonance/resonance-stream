//! Start-up: load config into the signals and, for a returning user, restore
//! history, start the sniffer and translator, and check for updates.

use crate::hooks::use_events::setup_event_listeners;
use crate::status_signals::{ServiceSignals, UpdateSignals};
use crate::store::AppSignals;
use crate::tauri_bridge::invoke;
use crate::ui_types::{AppConfig, ChatMessage, FolderStatus, SystemLogLevel, SystemMessage};
use crate::utils::add_system_log;
use crate::view_signals::ChatSignals;
use leptos::leptos_dom::log;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;

pub async fn hydrate_from_backend(signals: AppSignals) {
    let ServiceSignals {
        set_is_sniffer_active,
        set_model_ready,
        set_dict_update_available,
        ..
    } = signals.service;
    let UpdateSignals {
        set_show_app_update_modal,
        set_show_model_update_modal,
        set_pending_update_data,
        ..
    } = signals.updates;
    let ChatSignals {
        set_chat,
        set_system_log,
        ..
    } = signals.chat;
    let set_status_text = signals.setup.set_status_text;

    log!("App component hydration started...");
    // Load User Config
    match invoke("load_config", JsValue::NULL).await {
        Ok(res) => {
            if let Ok(config) = serde_wasm_bindgen::from_value::<AppConfig>(res) {
                log!("Loaded Config: {:?}", config);
                signals.config.apply(config.clone());

                // 2. If the user hasn't finished the wizard, stop here
                if config.init_done {
                    log!("Existing user detected. Auto-starting services.");
                    add_system_log(SystemLogLevel::Info, "Sniffer", "Auto-starting services...");
                    setup_event_listeners(signals).await;

                    // Hydrate GAME History
                    if let Ok(res) = invoke("get_chat_history", JsValue::NULL).await {
                        if let Ok(vec) = serde_wasm_bindgen::from_value::<Vec<ChatMessage>>(res) {
                            // Stickers/emotes arrive already normalized by the backend.
                            // Merged, not replaced: the listeners are already
                            // up, and messages that arrived during the fetch
                            // are in the store.
                            let limits = signals.config.tab_limits.get_untracked();
                            let filters = signals.config.custom_tab_filters.get_untracked();
                            let history = vec
                                .into_iter()
                                .map(|p| (p.pid, p.channel, ArcRwSignal::new(p)))
                                .collect();
                            set_chat.update(|store| {
                                store.merge_history(
                                    history,
                                    |m| m.with_untracked(|m| m.channel),
                                    &filters,
                                    &limits,
                                );
                                // Translations that beat their row to the store.
                                for (pid, text) in store.take_held_translations() {
                                    if let Some(row) = store.get(pid) {
                                        row.update(|c| c.translated = Some(text));
                                    }
                                }
                            });
                        }
                    }

                    // Hydrate SYSTEM History
                    if let Ok(res) = invoke("get_system_history", JsValue::NULL).await {
                        if let Ok(vec) = serde_wasm_bindgen::from_value::<Vec<SystemMessage>>(res) {
                            set_system_log.set(vec.into_iter().map(ArcRwSignal::new).collect());
                        }
                    }
                    set_is_sniffer_active.set(true);
                    let _ = invoke("start_sniffer_command", JsValue::NULL).await;

                    if config.use_translation {
                        let model = folder_exists("check_model_status").await;
                        let server = folder_exists("check_ai_server_status").await;
                        if model == Some(false) {
                            add_system_log(
                                SystemLogLevel::Warning,
                                "Sidecar",
                                "Model missing. AI is disabled.",
                            );
                        }
                        if server == Some(false) {
                            add_system_log(
                                SystemLogLevel::Warning,
                                "Sidecar",
                                "AI Server missing. AI is disabled.",
                            );
                        }
                        // The backend already started the translator at
                        // launch and reports its state through events.
                        match translator_ready(model, server) {
                            Some(true) => {
                                add_system_log(
                                    SystemLogLevel::Info,
                                    "UI",
                                    "Starting AI translation engine...",
                                );
                                set_model_ready.set(true);
                                set_status_text.set("AI Engine Starting...".to_string());
                            }
                            Some(false) => set_model_ready.set(false),
                            None => {}
                        }
                    }

                    if config.always_on_top {
                        let args = serde_wasm_bindgen::to_value(&serde_json::json!({
                            "onTop": true
                        }))
                        .unwrap();
                        let _ = invoke("set_always_on_top", args).await;
                    }

                    add_system_log(
                        SystemLogLevel::Info,
                        "Updater",
                        "Checking for remote updates...",
                    );
                    match invoke("check_all_updates", JsValue::NULL).await {
                        Ok(update_res) => {
                            if let Ok(update_data) = serde_wasm_bindgen::from_value::<
                                crate::ui_types::UpdateCheckResult,
                            >(update_res)
                            {
                                log!("data {:?}", update_data);

                                // 1. Silent Dictionary Update
                                // Only when auto-sync is on (off by default); the
                                // settings button and the wizard always sync.
                                if update_data.dict_update_available
                                    && signals.config.auto_sync_latest_dict.get_untracked()
                                {
                                    add_system_log(
                                        SystemLogLevel::Info,
                                        "Updater",
                                        "New dictionary found. Applying silently...",
                                    );
                                    let _ = invoke("sync_dictionary", JsValue::NULL).await;
                                }

                                set_dict_update_available.set(update_data.dict_update_available);

                                // 2. Save metadata for the modals to use
                                set_pending_update_data.set(Some(update_data.remote_data.clone()));

                                // 3. Trigger Popups
                                if update_data.app_update_available {
                                    set_show_app_update_modal.set(true);
                                }
                                if update_data.model_update_available && config.use_translation {
                                    set_show_model_update_modal.set(true);
                                }
                            }
                        }
                        Err(e) => {
                            log!("FATAL: check_all_updates failed: {:?}", e);
                            add_system_log(
                                SystemLogLevel::Error,
                                "Updater",
                                &format!("Check failed: {:?}", e),
                            );
                        }
                    }

                    set_status_text.set("Ready".to_string());
                } else {
                    log!("New user detected. Showing Wizard.");
                    add_system_log(
                        SystemLogLevel::Info,
                        "Setup",
                        "Awaiting initial configuration.",
                    );
                }
            }
        }
        Err(e) => log!("FATAL: Failed to load config: {:?}", e),
    }
}

/// Asks a `check_*_status` command whether its folder exists; `None` when
/// the call or its reply failed.
async fn folder_exists(command: &str) -> Option<bool> {
    let reply = invoke(command, JsValue::NULL).await.ok()?;
    serde_wasm_bindgen::from_value::<FolderStatus>(reply)
        .ok()
        .map(|status| status.exists)
}

/// Whether translation can start: model and server must both be there. A
/// failed check decides nothing, unless the other one already says missing.
fn translator_ready(model: Option<bool>, server: Option<bool>) -> Option<bool> {
    match (model, server) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_is_ready_only_when_model_and_server_both_exist() {
        // Regression (A5): the server check overwrote the model check, so a
        // missing model with the server present showed "starting".
        assert_eq!(translator_ready(Some(false), Some(true)), Some(false));
        assert_eq!(translator_ready(Some(true), Some(false)), Some(false));
        assert_eq!(translator_ready(Some(true), Some(true)), Some(true));
    }

    #[test]
    fn a_failed_check_decides_nothing_unless_the_other_says_missing() {
        assert_eq!(translator_ready(None, Some(true)), None);
        assert_eq!(translator_ready(Some(true), None), None);
        assert_eq!(translator_ready(None, Some(false)), Some(false));
    }
}
