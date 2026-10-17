use crate::store::{AppActions, AppSignals};
use crate::tauri_bridge::call;
use crate::ui_types::{TabSwitchModifier, Theme};
use leptos::prelude::*;
use leptos::reactive::spawn_local;
use resonance_types::ipc::{SetClickThrough, UpdateGlobalTabShortcut};

/// Click-through, drag-to-scroll, tab-switch shortcut, theme.
#[component]
pub fn AppearanceSection() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let actions = use_context::<AppActions>().expect("AppActions missing");

    view! {
        // ==========================================
        // SECTION: APPEARANCE
        // ==========================================
        <section class="space-y-4">

            // Click Through Mode
            <div class="form-control bg-base-200 p-3 rounded-lg border border-base-content/5">
                <label class="label cursor-pointer p-0 w-full justify-between gap-3">
                    <div class="flex flex-col">
                        <span class="label-text text-xs font-bold text-base-content/80">"클릭 관통 모드 (Click-Through)"</span>
                        <span class="text-[9px] text-warning mt-1">"주의: 비활성화 하려면 시스템 트레이(우측 하단 아이콘)를 사용하세요."</span>
                    </div>
                    <input type="checkbox" class="toggle toggle-success toggle-sm"
                        prop:checked=move || signals.ui.click_through.get()
                        on:change=move |ev| {
                            let enabled = event_target_checked(&ev);
                            signals.ui.set_click_through.set(enabled);
                            actions.save_config.dispatch(());
                            signals.ui.set_show_settings.set(false);

                            spawn_local(async move {
                                let _ = call(&SetClickThrough { enabled }).await;
                            });
                        }
                    />
                </label>
            </div>

            // --- DRAG TO SCROLL TOGGLE ---
            <div class="form-control bg-base-200 p-3 rounded-lg border border-base-content/5">
                <label class="label cursor-pointer p-0 w-full justify-between gap-3">
                    <div class="flex flex-col">
                        <span class="label-text text-xs font-bold text-base-content/80">"드래그 스크롤 (Drag to Scroll)"</span>
                        <span class="text-[9px] text-base-content/60 mt-1">"마우스로 채팅창 배경을 드래그하여 위아래로 스크롤합니다."</span>
                    </div>
                    <input type="checkbox" class="toggle toggle-success toggle-sm"
                        prop:checked=move || signals.config.drag_to_scroll.get()
                        on:change=move |ev| {
                            let enabled = event_target_checked(&ev);
                            signals.config.set_drag_to_scroll.set(enabled);
                            actions.save_config.dispatch(());
                        }
                    />
                </label>
            </div>

            // Tab Switch Shortcut
            <div class="form-control bg-base-200 p-3 rounded-lg border border-base-content/5">
                <label class="label p-0 mb-2">
                    <div class="flex flex-col">
                        <span class="label-text text-xs font-bold text-base-content/80">"탭 전환 단축키 (Tab Switch Shortcut)"</span>
                        <span class="text-[9px] text-base-content/60 mt-1">"버튼을 클릭하고 원하는 단축키 조합(예: Ctrl + Tab)을 누르세요."</span>
                    </div>
                </label>
                <div class="w-full flex justify-center items-center gap-1 mt-1">
                    <button class="btn btn-outline btn-sm w-48 font-bold focus:border-success focus:text-success focus:bg-success/10"
                        on:keydown=move |ev| {
                            ev.prevent_default();
                            let key_str = ev.key();

                            let ignored = ["Control", "Alt", "Shift", "Meta", "Escape", "CapsLock", "Process", "HangulMode", "HanjaMode"];

                            // Let users also press "Escape" to cancel/unbind!
                            if key_str == "Escape" {
                                signals.config.set_tab_switch_modifier.set(TabSwitchModifier::NoModifier);
                                signals.config.set_tab_switch_key.set("".to_string());
                                actions.save_config.dispatch(());

                                spawn_local(async move {
                                    let _ = call(&UpdateGlobalTabShortcut {
                                        modifier: TabSwitchModifier::NoModifier.as_str().to_string(),
                                        key: String::new(),
                                    }).await;
                                });
                                return;
                            }

                            if !ignored.contains(&key_str.as_str()) {
                                let modifier = if ev.ctrl_key() || ev.meta_key() {
                                    TabSwitchModifier::Ctrl
                                } else if ev.alt_key() {
                                    TabSwitchModifier::Alt
                                } else if ev.shift_key() {
                                    TabSwitchModifier::Shift
                                } else {
                                    TabSwitchModifier::NoModifier
                                };

                                signals.config.set_tab_switch_modifier.set(modifier);
                                signals.config.set_tab_switch_key.set(key_str.clone());
                                actions.save_config.dispatch(());

                                let rust_mod = modifier.as_str();
                                let rust_key = key_str.clone();

                                spawn_local(async move {
                                    let _ = call(&UpdateGlobalTabShortcut {
                                        modifier: rust_mod.to_string(),
                                        key: rust_key,
                                    }).await;
                                });
                            }
                        }
                    >
                        {move || {
                            let m = signals.config.tab_switch_modifier.get();
                            let k = signals.config.tab_switch_key.get();

                            let key_display = match k.as_str() {
                                " " => "Space".to_string(),
                                "ArrowRight" => "→ (Right)".to_string(),
                                "ArrowLeft" => "← (Left)".to_string(),
                                "ArrowUp" => "↑ (Up)".to_string(),
                                "ArrowDown" => "↓ (Down)".to_string(),
                                "" => "지정되지 않음".to_string(), // Better text for empty state
                                other => {
                                    let mut c = other.chars();
                                    match c.next() {
                                        None => String::new(),
                                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                                    }
                                }
                            };

                            if m == TabSwitchModifier::NoModifier {
                                key_display
                            } else {
                                format!("{} + {}", m.as_str(), key_display)
                            }
                        }}
                    </button>

                    // --- NEW: DEDICATED UNBIND BUTTON ---
                    <div class="tooltip tooltip-top" data-tip="단축키 해제">
                        <button class="btn btn-outline btn-sm btn-error w-8 p-0 font-black focus:outline-none"
                            on:click=move |_| {
                                signals.config.set_tab_switch_modifier.set(TabSwitchModifier::NoModifier);
                                signals.config.set_tab_switch_key.set("".to_string());
                                actions.save_config.dispatch(());

                                spawn_local(async move {
                                    // Sending empty strings unregisters the current key without adding a new one
                                    let _ = call(&UpdateGlobalTabShortcut {
                                        modifier: TabSwitchModifier::NoModifier.as_str().to_string(),
                                        key: String::new(),
                                    }).await;
                                });
                            }
                        >
                            "✕"
                        </button>
                    </div>
                </div>
                <div class="text-[9px] text-warning mt-2 bg-warning/10 p-1.5 rounded">
                    "⚠️ 주의: Ctrl+A, Ctrl+C 같은 윈도우 기본 단축키나 게임 내 필수 키를 등록하면, 해당 키의 원래 기능이 작동하지 않게 됩니다. (Ctrl + Tab 또는 Ctrl + `(1옆에) 조합을 권장합니다)"
                </div>
            </div>

            // Theme Toggle
            <button class="btn btn-sm btn-block justify-between bg-base-200 border-base-content/5 font-bold hover:bg-base-content/10"
                    on:click=move |_| {
                        signals.config.set_theme.set(signals.config.theme.get().toggled());
                        actions.save_config.dispatch(());
                    }>
                <span class="text-xs">"Theme Mode"</span>
                <span class="text-[10px] uppercase tracking-widest opacity-70">
                    {move || if signals.config.theme.get() == Theme::Dark { "🌙 Dark" } else { "☀️ Light" }}
                </span>
            </button>
        </section>
    }
}
