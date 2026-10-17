//! "Update available" modals for the app and the AI model.

use crate::download_progress::{bar_label, bar_value, is_cancelled, RELEASES_URL};
use crate::status_signals::UpdateSignals;
use crate::store::AppSignals;
use crate::tauri_bridge::{invoke, listen};
use crate::ui_types::ProgressPayload;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;

/// What the backend said went wrong: Tauri rejects a command with its error
/// string.
fn error_text(error: &JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{:?}", error))
}

/// App update: release notes, skip, download with progress, restart to apply;
/// a failed download or signature check ends in an error step (3) with a retry.
#[component]
pub fn AppUpdateModal() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let UpdateSignals {
        show_app_update_modal,
        set_show_app_update_modal,
        pending_update_data,
        app_update_step,
        set_app_update_step,
        app_update_progress,
        set_app_update_progress,
        app_update_error,
        set_app_update_error,
        ..
    } = signals.updates;
    let set_status_text = signals.setup.set_status_text;

    // --- APP UPDATE LOGIC ---
    let start_app_update = move |download_url: String| {
        set_app_update_step.set(1);
        set_app_update_progress.set(0);

        spawn_local(async move {
            let progress_closure = Closure::wrap(Box::new(move |event_obj: JsValue| {
                if let Ok(ev) = serde_wasm_bindgen::from_value::<serde_json::Value>(event_obj) {
                    if let Ok(payload) =
                        serde_json::from_value::<ProgressPayload>(ev["payload"].clone())
                    {
                        if payload.current_file.contains("앱 업데이트") {
                            // Only the bar: "done" is the command succeeding,
                            // i.e. the signature check passing, not 100 % of
                            // the bytes.
                            set_app_update_progress.set(payload.percent);
                        }
                    }
                }
            }) as Box<dyn FnMut(JsValue)>);

            listen("download-progress", &progress_closure).await;
            progress_closure.forget(); // Keep alive during download

            let args =
                serde_wasm_bindgen::to_value(&serde_json::json!({ "downloadUrl": download_url }))
                    .unwrap();
            match invoke("download_app_update", args).await {
                Ok(_) => set_app_update_step.set(2),
                Err(e) => {
                    let reason = error_text(&e);
                    // A cancel already put the dialog back to step 0.
                    if !is_cancelled(&reason) {
                        set_app_update_error.set(reason);
                        set_app_update_step.set(3);
                    }
                }
            }
        });
    };

    view! {
        // ==========================================
        // APP UPDATE MODAL
        // ==========================================
        <Show when=move || show_app_update_modal.get()>
            <div class="modal modal-open backdrop-blur-sm z-[30000]">
                <div class="modal-box bg-base-300 border border-success/30">
                    <h3 class="font-black text-lg text-success mb-4">"새로운 앱 업데이트 가능!"</h3>
                    {move || pending_update_data.get().map(|data| view! {
                        <div class="space-y-4">
                            {move || match app_update_step.get() {
                                0 => view! {
                                    <div class="animate-in fade-in">
                                        <p class="text-sm font-bold">"버전: " {data.app.latest_version.clone()}</p>
                                        <div class="bg-base-200 p-3 rounded text-xs opacity-80 whitespace-pre-wrap mt-2 max-h-48 overflow-y-auto">
                                            {data.app.release_notes.clone()}
                                        </div>
                                        <div class="modal-action">
                                            <button class="btn btn-ghost text-base-content/50"
                                                on:click={
                                                    // CLONE DATA BEFORE THE CLOSURE
                                                    let version = data.app.latest_version.clone();
                                                    move |_| {
                                                        let v = version.clone();
                                                        spawn_local(async move {
                                                            let args = serde_wasm_bindgen::to_value(&serde_json::json!({
                                                                "target": "app",
                                                                "version": v
                                                            })).unwrap();
                                                            let _ = invoke("ignore_update", args).await;
                                                        });
                                                        set_show_app_update_modal.set(false);
                                                    }
                                                }>
                                                "이번 버전 건너뛰기"
                                            </button>
                                            <button class="btn btn-success"
                                                on:click={
                                                    // CLONE DATA BEFORE THE CLOSURE
                                                    let url = data.app.download_url.clone();
                                                    move |_| { start_app_update(url.clone()); }
                                                }>
                                                "다운로드 시작"
                                            </button>
                                        </div>
                                    </div>
                                }.into_any(),

                                1 => view! {
                                    <div class="space-y-2 py-4 animate-in fade-in text-center">
                                        <p class="text-sm font-bold opacity-80">"업데이트 파일을 다운로드 중입니다..."</p>
                                        // No value until the first percent: a moving bar, not a stuck "0%".
                                        <progress class="progress progress-success w-full h-4"
                                            value=move || bar_value(app_update_progress.get()).map(|v| v.to_string())
                                            max="100"></progress>
                                        <span class="text-xs font-mono">{move || bar_label(app_update_progress.get())}</span>
                                        <div class="modal-action justify-center">
                                            <button class="btn btn-ghost btn-sm text-base-content/50"
                                                on:click=move |_| {
                                                    spawn_local(async move {
                                                        // Returns once the download has let go of its file.
                                                        let _ = invoke("cancel_app_update", JsValue::NULL).await;
                                                        set_app_update_progress.set(0);
                                                        set_app_update_step.set(0);
                                                    });
                                                }>
                                                "취소"
                                            </button>
                                        </div>
                                    </div>
                                }.into_any(),

                                2 => view! {
                                    <div class="space-y-4 py-4 animate-in zoom-in text-center">
                                        <div class="text-4xl mb-2">"🎉"</div>
                                        <p class="text-lg font-bold text-success">"다운로드 완료!"</p>
                                        <p class="text-xs opacity-70">"서명 확인을 마쳤습니다. 새로운 버전을 적용하려면 앱을 재시작해야 합니다."</p>
                                        <button class="btn btn-success btn-block mt-4 gap-2"
                                            on:click=move |_| {
                                                set_status_text.set("재시작 중...".to_string());
                                                spawn_local(async move {
                                                    // On success the app exits and nothing comes back.
                                                    if let Err(e) = invoke("restart_to_apply_update", JsValue::NULL).await {
                                                        set_app_update_error.set(error_text(&e));
                                                        set_app_update_step.set(3);
                                                    }
                                                });
                                            }>
                                            "재시작 및 적용"
                                        </button>
                                    </div>
                                }.into_any(),

                                _ => view! {
                                    <div class="space-y-3 py-2 animate-in fade-in">
                                        <p class="text-sm font-bold text-error">"업데이트를 설치하지 않았습니다."</p>
                                        <p class="text-xs opacity-70">
                                            "받은 파일이 올바르지 않거나 다운로드에 실패했습니다. 현재 버전은 그대로 사용할 수 있습니다."
                                        </p>
                                        <div class="bg-base-200 p-3 rounded text-xs font-mono opacity-80 break-words">
                                            {move || app_update_error.get()}
                                        </div>
                                        <div class="modal-action">
                                            <button class="btn btn-ghost text-base-content/50"
                                                on:click=move |_| {
                                                    set_show_app_update_modal.set(false);
                                                    set_app_update_step.set(0);
                                                    set_app_update_error.set(String::new());
                                                }>
                                                "닫기"
                                            </button>
                                            <button class="btn btn-outline"
                                                on:click=move |_| {
                                                    spawn_local(async move {
                                                        let args = serde_wasm_bindgen::to_value(&serde_json::json!({ "url": RELEASES_URL })).unwrap();
                                                        let _ = invoke("open_browser", args).await;
                                                    });
                                                }>
                                                "직접 다운로드"
                                            </button>
                                            <button class="btn btn-success"
                                                on:click={
                                                    let url = data.app.download_url.clone();
                                                    move |_| {
                                                        set_app_update_error.set(String::new());
                                                        start_app_update(url.clone());
                                                    }
                                                }>
                                                "다시 시도"
                                            </button>
                                        </div>
                                    </div>
                                }.into_any(),
                            }}
                        </div>
                    })}
                </div>
            </div>
        </Show>
    }
}

/// AI model update: release notes, skip, download with progress.
#[component]
pub fn ModelUpdateModal() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let UpdateSignals {
        show_model_update_modal,
        set_show_model_update_modal,
        pending_update_data,
        model_update_step,
        set_model_update_step,
        model_update_progress,
        set_model_update_progress,
        ..
    } = signals.updates;

    // --- MODEL UPDATE LOGIC ---
    let start_model_update = move || {
        set_model_update_step.set(1);
        set_model_update_progress.set(0);

        spawn_local(async move {
            let progress_closure = Closure::wrap(Box::new(move |event_obj: JsValue| {
                if let Ok(ev) = serde_wasm_bindgen::from_value::<serde_json::Value>(event_obj) {
                    if let Ok(payload) =
                        serde_json::from_value::<ProgressPayload>(ev["payload"].clone())
                    {
                        if payload.current_file.contains("AI 모델") {
                            set_model_update_progress.set(payload.percent);
                            if payload.percent >= 100 {
                                set_model_update_step.set(2);
                            }
                        }
                    }
                }
            }) as Box<dyn FnMut(JsValue)>);

            listen("download-progress", &progress_closure).await;
            progress_closure.forget(); // Keep alive during download

            // The backend takes the model's address and hash from the verified metadata.
            let _ = invoke("download_model", JsValue::NULL).await;
        });
    };

    view! {
        // ==========================================
        // MODEL UPDATE MODAL
        // ==========================================
        <Show when=move || show_model_update_modal.get()>
            <div class="modal modal-open backdrop-blur-sm z-[30000]">
                <div class="modal-box bg-base-300 border border-info/30">
                    <h3 class="font-black text-lg text-info mb-4">"새로운 AI 번역 모델!"</h3>
                    {move || pending_update_data.get().map(|data| view! {
                        <div class="space-y-4">
                            {move || match model_update_step.get() {
                                0 => view! {
                                    <div class="animate-in fade-in">
                                        <p class="text-sm font-bold">"버전: " {data.model.latest_version.clone()}</p>
                                        <div class="bg-base-200 p-3 rounded text-xs opacity-80 whitespace-pre-wrap mt-2 max-h-48 overflow-y-auto">
                                            {data.model.release_notes.clone()}
                                        </div>
                                        <div class="modal-action">
                                            <button class="btn btn-ghost text-base-content/50"
                                                on:click={
                                                    // CLONE DATA BEFORE THE CLOSURE
                                                    let version = data.model.latest_version.clone();
                                                    move |_| {
                                                        let v = version.clone();
                                                        spawn_local(async move {
                                                            let args = serde_wasm_bindgen::to_value(&serde_json::json!({
                                                                "target": "model",
                                                                "version": v
                                                            })).unwrap();
                                                            let _ = invoke("ignore_update", args).await;
                                                        });
                                                        set_show_model_update_modal.set(false);
                                                    }
                                                }>
                                                "건너뛰기"
                                            </button>
                                            <button class="btn btn-info"
                                                on:click=move |_| start_model_update()>
                                                "다운로드 시작 (약 2.4GB)"
                                            </button>
                                        </div>
                                    </div>
                                }.into_any(),

                                1 => view! {
                                    <div class="space-y-2 py-4 animate-in fade-in text-center">
                                        <p class="text-sm font-bold opacity-80">"AI 모델을 다운로드 중입니다..."</p>
                                        <progress class="progress progress-info w-full h-4" value=move || model_update_progress.get().to_string() max="100"></progress>
                                        <span class="text-xs font-mono">{move || format!("{}%", model_update_progress.get())}</span>
                                    </div>
                                }.into_any(),

                                _ => view! {
                                    <div class="space-y-4 py-4 animate-in zoom-in text-center">
                                        <div class="text-4xl mb-2">"✨"</div>
                                        <p class="text-lg font-bold text-info">"모델 다운로드 완료!"</p>
                                        <p class="text-xs opacity-70">"새로운 AI 모델이 디스크에 성공적으로 저장되었습니다."</p>
                                        <button class="btn btn-info btn-block mt-4 gap-2"
                                            on:click=move |_| {
                                                set_show_model_update_modal.set(false);
                                                signals.service.set_restart_required.set(true);
                                            }>
                                            "확인"
                                        </button>
                                    </div>
                                }.into_any(),
                            }}
                        </div>
                    })}
                </div>
            </div>
        </Show>
    }
}
