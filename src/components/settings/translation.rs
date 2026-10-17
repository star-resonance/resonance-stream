use crate::store::{AppActions, AppSignals};
use crate::tauri_bridge::call;
use crate::ui_types::{ComputeMode, Tier};
use leptos::prelude::*;
use leptos::reactive::spawn_local;
use resonance_types::ipc::{CheckAiServerStatus, CheckModelStatus};

/// AI translation on/off, compute mode and VRAM tier.
#[component]
pub fn TranslationSection() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let actions = use_context::<AppActions>().expect("AppActions missing");

    view! {
        // ==========================================
        // SECTION: AI TRANSLATION
        // ==========================================
        <section class="space-y-3">

            <div class="form-control">
                <label class="label cursor-pointer w-full justify-between gap-3 bg-base-100 rounded-lg px-4 py-3 border border-base-content/5 hover:border-success/30 transition-all">
                    <span class="label-text font-bold text-base-content">"실시간 번역 기능 사용"</span>
                    <input type="checkbox" class="toggle toggle-success toggle-sm"
                        prop:checked=move || signals.config.use_translation.get()
                        on:click=move |ev| {
                            // Prevent the browser from automatically flipping the switch
                            let is_turning_on = event_target_checked(&ev);

                            if is_turning_on {
                                // 1. Optimistically set the UI to ON so the toggle moves immediately
                                signals.config.set_use_translation.set(true);

                                spawn_local(async move {
                                    let mut has_error = false;

                                    // 2. Check Model Status
                                    if let Ok(status) = call(&CheckModelStatus {}).await {
                                        if !status.exists {
                                            has_error = true;
                                            if let Some(w) = web_sys::window() {
                                                if w.confirm_with_message("AI 모델 파일이 없습니다. 다운로드 화면으로 이동하시겠습니까?").unwrap_or(false) {
                                                    signals.setup.set_wizard_step.set(2);
                                                    signals.ui.set_show_settings.set(false);
                                                    signals.config.set_init_done.set(false);
                                                }
                                            }
                                        }
                                    }

                                    // 3. Check Server Status (Only if model check passed, preventing double popups!)
                                    if !has_error {
                                        if let Ok(status) = call(&CheckAiServerStatus {}).await {
                                            if !status.exists {
                                                has_error = true;
                                                if let Some(w) = web_sys::window() {
                                                    if w.confirm_with_message("AI 실행 파일이 없습니다. 다운로드 화면으로 이동하시겠습니까?").unwrap_or(false) {
                                                        signals.setup.set_wizard_step.set(2);
                                                        signals.ui.set_show_settings.set(false);
                                                        signals.config.set_init_done.set(false);
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // 4. Finalize
                                    if has_error {
                                        // Revert the toggle visually back to OFF if files are missing
                                        signals.config.set_use_translation.set(false);
                                    } else {
                                        // Everything exists, safely save the config
                                        actions.save_config.dispatch(());
                                    }
                                });
                            } else {
                                // User is turning it OFF (Toggle visually moves immediately)
                                signals.config.set_use_translation.set(false);
                                actions.save_config.dispatch(());
                            }
                        }
                    />
                </label>
            </div>

            <Show when=move || signals.config.use_translation.get()>
                // Compute Mode Radio Group
                <div class="p-3 bg-base-200 rounded-lg space-y-3 border border-base-content/5">
                    <span class="text-[11px] font-bold text-base-content/50 uppercase">"연산 장치 (Compute Mode)"</span>
                    <div class="join w-full">
                        {ComputeMode::ALL.iter().copied().map(|m| {
                            view! {
                                <button
                                    class="join-item btn btn-xs flex-1 font-black border-base-content/10"
                                    class:btn-success=move || signals.config.compute_mode.get() == m
                                    class:btn-outline=move || signals.config.compute_mode.get() != m
                                    on:click=move |_| {
                                        signals.config.set_compute_mode.set(m);
                                        actions.save_config.dispatch(());
                                        signals.service.set_restart_required.set(true);
                                    }
                                >
                                    {m.label()}
                                </button>
                            }
                        }).collect_view()}
                    </div>

                    // Hide VRAM settings if CPU is selected
                    <Show when=move || signals.config.compute_mode.get() == ComputeMode::Gpu>
                        <span class="text-[11px] font-bold text-base-content/50 uppercase block mt-3">"VRAM 사용량 (GPU Offload)"</span>
                        <div class="join w-full">
                            {Tier::ALL.iter().copied().map(|t| {
                                view! {
                                    <button
                                        class="join-item btn btn-xs flex-1 font-black border-base-content/10"
                                        class:btn-success=move || signals.config.tier.get() == t
                                        class:btn-outline=move || signals.config.tier.get() != t
                                        on:click=move |_| {
                                            signals.config.set_tier.set(t);
                                            actions.save_config.dispatch(());
                                            signals.service.set_restart_required.set(true);
                                        }
                                    >
                                        {t.label()}
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                        // Updated the description to accurately reflect that it improves speed, not quality
                        <div class="text-[9px] opacity-50">"할당량이 높을수록 번역 속도가 빨라지지만 VRAM을 더 많이 소모합니다."</div>
                    </Show>

                    // Catch-up: missed messages translated at each translator start
                    <div class="flex items-center justify-between mt-3">
                        <div class="flex flex-col">
                            <span class="text-[11px] font-bold text-base-content/50 uppercase">"놓친 메시지 번역 (Catch-up)"</span>
                            <span class="text-[9px] opacity-50">"번역기가 시작될 때 앱 실행 후 번역되지 않은 최근 메시지를 번역합니다. 0 = 끄기"</span>
                        </div>
                        <div class="flex items-center gap-1">
                            <input type="number" min="0" max="1000" step="10"
                                class="input input-xs input-bordered w-16 text-right"
                                prop:value=move || signals.config.translation_catch_up_limit.get().to_string()
                                on:change=move |ev| {
                                    let limit = event_target_value(&ev)
                                        .trim()
                                        .parse::<usize>()
                                        .unwrap_or_else(|_| signals.config.translation_catch_up_limit.get_untracked())
                                        .min(1000);
                                    signals.config.set_translation_catch_up_limit.set(limit);
                                    actions.save_config.dispatch(());
                                }
                            />
                            <span class="text-[10px] opacity-60">"개"</span>
                        </div>
                    </div>

                    <Show when=move || signals.service.restart_required.get()>
                        <div class="text-[10px] text-warning font-bold animate-pulse mt-2 p-2 bg-warning/10 rounded">
                            "⚠️ 변경 사항 적용을 위해 AI 번역기가 재시작 됩니다. 번역을 위해 잠시 시간이 소요됩니다."
                        </div>
                    </Show>
                </div>
            </Show>
        </section>
    }
}
