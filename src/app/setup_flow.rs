//! First-run wizard flow: download model + server + dictionary, then start services.

use crate::config_signals::ConfigSignals;
use crate::hooks::use_events::setup_event_listeners;
use crate::setup_plan::{check_failed_message, plan_downloads};
use crate::status_signals::{ServiceSignals, SetupSignals};
use crate::store::AppSignals;
use crate::tauri_bridge::{invoke, listen};
use crate::ui_types::{SystemLogLevel, TauriEvent};
use crate::utils::add_system_log;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;

/// Marks setup done, saves config, and starts the event listeners and sniffer.
pub fn finalize_setup(
    signals: AppSignals,
    save_config: Action<(), ()>,
) -> impl Fn(()) + Copy + Send + Sync + 'static {
    let ConfigSignals { set_init_done, .. } = signals.config;
    let ServiceSignals {
        set_is_sniffer_active,
        ..
    } = signals.service;

    move |_| {
        set_init_done.set(true);
        add_system_log(
            SystemLogLevel::Success,
            "Setup",
            "Initial configuration completed.",
        );
        save_config.dispatch(());

        spawn_local(async move {
            add_system_log(
                SystemLogLevel::Info,
                "Sniffer",
                "Initializing packet capture...",
            );
            setup_event_listeners(signals).await;
            set_is_sniffer_active.set(true);
            let _ = invoke("start_sniffer_command", JsValue::NULL).await;
        });
    }
}

/// Fetches the signed model / dictionary metadata, downloads model, server and dictionary with
/// progress, launches the translator, then calls `finalize`. Every way it stops leaves a line under
/// the button (`setup_error`), as well as the status text and the system log.
pub fn start_download(
    signals: AppSignals,
    finalize_setup: impl Fn(()) + Copy + Send + Sync + 'static,
) -> impl Fn(web_sys::MouseEvent) + Copy + Send + Sync + 'static {
    let SetupSignals {
        set_status_text,
        set_downloading,
        set_progress,
        set_setup_error,
        ..
    } = signals.setup;
    let ServiceSignals {
        set_model_ready, ..
    } = signals.service;

    // The wizard stopped: back to the button, with the reason on screen and in the log.
    let stop = move |status: String, message: String| {
        set_downloading.set(false);
        set_status_text.set(status);
        set_setup_error.set(Some(message.clone()));
        add_system_log(SystemLogLevel::Error, "ModelManager", &message);
    };
    // What the backend said, as text (`invoke` gives a string for a command's `Err(String)`).
    let reason = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));

    move |ev: web_sys::MouseEvent| {
        // Prevent the default button behavior if necessary
        ev.prevent_default();

        set_downloading.set(true);
        set_setup_error.set(None);
        set_status_text.set("Starting Downloads...".to_string());

        spawn_local(async move {
            // FETCH THE SIGNED METADATA FIRST
            let update_res = invoke("check_all_updates", JsValue::NULL).await;
            let check = match update_res {
                Ok(res) => {
                    match serde_wasm_bindgen::from_value::<crate::ui_types::UpdateCheckResult>(res)
                    {
                        Ok(check) => check,
                        Err(_) => {
                            stop(
                                "Error: Failed to parse update data".to_string(),
                                "업데이트 정보를 읽지 못했습니다.".to_string(),
                            );
                            return;
                        }
                    }
                }
                Err(e) => {
                    stop(
                        "Error: Network check failed".to_string(),
                        check_failed_message(&reason(e)),
                    );
                    return;
                }
            };
            if let Err(message) = plan_downloads(&check) {
                stop("Error: model information refused".to_string(), message);
                return;
            }

            // 1. Setup the progress listener
            let closure = Closure::wrap(Box::new(move |event_obj: JsValue| {
                if let Ok(wrapper) = serde_wasm_bindgen::from_value::<TauriEvent>(event_obj) {
                    set_progress.set(wrapper.payload.total_percent);
                    // Optional: Update status text to show what is currently downloading
                    // using the `current_file` field we defined in downloader.rs
                    set_status_text.set(format!(
                        "{} ({}%)",
                        wrapper.payload.current_file, wrapper.payload.total_percent
                    ));
                }
            }) as Box<dyn FnMut(JsValue)>);
            let _ = listen("download-progress", &closure).await;

            // 2. Download the AI Model (.gguf): the backend takes its address and hash from the
            // verified metadata.
            if let Err(e) = invoke("download_model", JsValue::NULL).await {
                let why = reason(e);
                stop(
                    format!("Model Error: {why}"),
                    format!("모델 다운로드에 실패했습니다. ({why})"),
                );
                closure.forget();
                return;
            }

            // 3. Download the AI Server (llama-server.exe via zip)
            if let Err(e) = invoke("download_ai_server", JsValue::NULL).await {
                let why = reason(e);
                stop(
                    format!("Server Error: {why}"),
                    format!("AI 서버 다운로드에 실패했습니다. ({why})"),
                );
                closure.forget();
                return;
            }

            // 4. Sync dictionary
            if let Err(e) = invoke("sync_dictionary", JsValue::NULL).await {
                let why = reason(e);
                stop(
                    format!("Dict Error: {why}"),
                    format!("사용자 사전 동기화에 실패했습니다. ({why})"),
                );
                closure.forget();
                return;
            }

            // The translator starts when finalize saves the config (translation
            // goes off -> on there); starting it here too ran two servers.

            // 5. Both downloads succeeded
            set_downloading.set(false);
            set_model_ready.set(true);
            set_status_text.set("Ready".to_string());
            finalize_setup(());

            closure.forget();
        });
    }
}
