use crate::store::{AppActions, AppSignals};
use leptos::prelude::*;

/// Font size, message limit, compact/relative-time toggles, minimum sender level.
#[component]
pub fn ChatSection() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let actions = use_context::<AppActions>().expect("AppActions missing");

    view! {
        // ==========================================
        // SECTION: CHAT SETTINGS
        // ==========================================
        <section class="space-y-4">

            // Font Size Slider
            <div class="space-y-2 mt-4 pt-4 border-t border-base-content/10">
                <div class="flex justify-between text-[11px] font-bold">
                    <span class="text-base-content/80">"채팅 글꼴 크기 (Font Size)"</span>
                    <span class="text-success">{move || format!("{}px", signals.config.font_size.get())}</span>
                </div>
                <input type="range" min="10" max="24" step="1"
                    class="range range-xs range-success w-full"
                    prop:value=move || signals.config.font_size.get().to_string()
                    on:input=move |ev| {
                        // 1. Update UI live while dragging
                        let val = event_target_value(&ev).parse::<u32>().unwrap_or(14);
                        signals.config.set_font_size.set(val);
                    }
                    on:change=move |_| {
                        // 2. Save to file when mouse is released
                        actions.save_config.dispatch(());
                    }
                />
                <div class="text-[9px] text-base-content/50">"기본 크기는 14px 입니다."</div>
            </div>

            // Message Spacing Slider
            <div class="space-y-2 mt-4 pt-4 border-t border-base-content/10">
                <div class="flex justify-between text-[11px] font-bold">
                    <span class="text-base-content/80">"메시지 간격 (Message Spacing)"</span>
                    <span class="text-success">{move || format!("{}px", signals.config.message_spacing.get())}</span>
                </div>
                <input type="range" min="0" max="24" step="1"
                    class="range range-xs range-success w-full"
                    prop:value=move || signals.config.message_spacing.get().to_string()
                    on:input=move |ev| {
                        // Update UI live while dragging
                        let val = event_target_value(&ev).parse::<u32>().unwrap_or(4);
                        signals.config.set_message_spacing.set(val);
                    }
                    on:change=move |ev| {
                        // Save to config when released
                        let val = event_target_value(&ev).parse::<u32>().unwrap_or(4);
                        signals.config.set_message_spacing.set(val);
                        actions.save_config.dispatch(());
                    }
                />
                <div class="text-[9px] text-base-content/50">"채팅 메시지 위아래의 여백을 조절합니다. (기본값: 4px)"</div>
            </div>

            <div class="form-control bg-base-200 p-3 rounded-lg border border-base-content/5">
                <label class="label cursor-pointer p-0 w-full justify-between gap-3">
                    <span class="label-text text-xs font-bold text-base-content/80">"컴팩트 모드에서 번역 시 원문 숨기기"</span>
                    <input type="checkbox" class="toggle toggle-success toggle-sm"
                        prop:checked=move || signals.config.hide_original_in_compact.get()
                        on:change=move |ev| {
                            signals.config.set_hide_original_in_compact.set(event_target_checked(&ev));
                            actions.save_config.dispatch(());
                        }
                    />
                </label>
            </div>

            // Relative Time Toggle
            <div class="form-control bg-base-200 p-3 rounded-lg border border-base-content/5">
                <label class="label cursor-pointer p-0 w-full justify-between gap-3">
                    <div class="flex flex-col">
                        <span class="label-text text-xs font-bold text-base-content/80">"상대적 시간 표시 (Relative Time)"</span>
                        <span class="text-[9px] text-base-content/60 mt-1">"시간을 'now', '4m' 형식으로 표시합니다."</span>
                    </div>
                    <input type="checkbox" class="toggle toggle-success toggle-sm"
                        prop:checked=move || signals.config.use_relative_time.get()
                        on:change=move |ev| {
                            signals.config.set_use_relative_time.set(event_target_checked(&ev));
                            actions.save_config.dispatch(());
                        }
                    />
                </label>
            </div>

            // In-game chat sending (opt-in)
            <div class="form-control bg-base-200 p-3 rounded-lg border border-base-content/5">
                <label class="label cursor-pointer p-0 w-full justify-between gap-3">
                    <div class="flex flex-col">
                        <span class="label-text text-xs font-bold text-base-content/80">"게임 내 채팅 보내기"</span>
                        <span class="text-[9px] text-base-content/60 mt-1">"게임 프로세스에 전송 모듈을 불러와 채팅을 보냅니다. 게임 이용약관에 어긋날 수 있으며, 사용에 따른 책임은 사용자에게 있습니다."</span>
                    </div>
                    <input type="checkbox" class="toggle toggle-success toggle-sm"
                        prop:checked=move || signals.config.ingame_send.get()
                        on:change=move |ev| {
                            signals.config.set_ingame_send.set(event_target_checked(&ev));
                            actions.save_config.dispatch(());
                        }
                    />
                </label>
            </div>

            // Minimum Sender Level Filter (Spam Prevention)
            <div class="space-y-2 mt-4 pt-4 border-t border-base-content/10">
                <div class="flex justify-between text-[11px] font-bold">
                    <div class="flex flex-col">
                        <span class="text-base-content/80">"생체 엔그렘 레벨"</span>
                    </div>
                    <span class="text-success">{move || format!("Lv. {}", signals.config.min_sender_level.get())}</span>
                </div>
                <input type="range" min="1" max="60" step="1"
                    class="range range-xs range-success w-full"
                    prop:value=move || signals.config.min_sender_level.get().to_string()
                    on:input=move |ev| {
                        // Update UI live while dragging
                        let val = event_target_value(&ev).parse::<u64>().unwrap_or(1);
                        signals.config.set_min_sender_level.set(val);
                    }
                    on:change=move |ev| {
                        // Save to config when released
                        let val = event_target_value(&ev).parse::<u64>().unwrap_or(1);
                        signals.config.set_min_sender_level.set(val);
                        actions.save_config.dispatch(());
                    }
                />
                <div class="text-[9px] text-base-content/50">"설정한 레벨 미만의 유저가 보낸 채팅은 화면에 표시되지 않습니다. (스팸 봇 차단용)"</div>
            </div>
        </section>
    }
}
