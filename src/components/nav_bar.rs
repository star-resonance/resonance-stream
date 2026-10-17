use crate::chat_view::Tab;
use crate::components::icons::{self, icon};
use crate::readability::{
    nav_bar_bg, tab_label_class, WINDOW_BUTTON, WINDOW_BUTTON_ACTIVE, WINDOW_BUTTON_CLOSE,
};
use crate::store::{AppActions, AppSignals};
use crate::tauri_bridge::call;
use crate::ui_types::{default_tab_limit, Channel, SystemLogLevel};
use crate::utils::add_system_log;
use leptos::ev::{click, keydown};
use leptos::html::{Button, Div, Input};
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::{event, CloseWindow, OpenPopup, SetAlwaysOnTop};
use resonance_types::PopupKind;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsValue;
use web_sys::Node;

/// One button of the bar's right-hand group: a flat 40 px box with an icon,
/// green while `active`. Every button there -- search, cheat sheet, favorites,
/// pin, the tools, expand -- is this one component, so they all share a width.
#[component]
fn NavButton<F>(
    title: &'static str,
    on_click: F,
    /// Lit up (green): the pin is on, a search is set, the tools are open.
    #[prop(into, optional)]
    active: Signal<bool>,
    #[prop(optional)] button_ref: NodeRef<Button>,
    children: Children,
) -> impl IntoView
where
    F: Fn(web_sys::MouseEvent) + 'static,
{
    view! {
        <button
            node_ref=button_ref
            class=move || if active.get() { WINDOW_BUTTON_ACTIVE } else { WINDOW_BUTTON }
            title=title
            on:click=on_click
        >
            {children()}
        </button>
    }
}

#[component]
pub fn NavBar() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let actions = use_context::<AppActions>().expect("AppActions missing");

    let (is_search_open, set_is_search_open) = signal(false);
    let (is_controls_open, set_is_controls_open) = signal(false);
    let (context_menu_open, set_context_menu_open) = signal(None::<String>);

    // The buttons both modes show -- cheat sheet, favorites, pin -- share their handlers.
    // Both open in a window of their own (`open_popup`).
    let open_popup = |kind: PopupKind| {
        spawn_local(async move {
            if let Err(e) = call(&OpenPopup { kind }).await {
                add_system_log(SystemLogLevel::Error, "Popup", &e);
            }
        });
    };
    let open_cheatsheet = move |_| open_popup(PopupKind::CheatSheet);
    let open_favorites = move |_| open_popup(PopupKind::Favorites);
    let toggle_pin = move |_| {
        let new_state = !signals.config.always_on_top.get();
        signals.config.set_always_on_top.set(new_state);
        spawn_local(async move {
            let _ = call(&SetAlwaysOnTop { on_top: new_state }).await;
        });
        actions.save_config.dispatch(());
    };

    // --- NODE REFERENCES ---
    let search_input_ref = NodeRef::<Input>::new();
    let search_container_ref = NodeRef::<Div>::new();
    let search_btn_ref = NodeRef::<Button>::new();
    let controls_container_ref = NodeRef::<Div>::new();
    let folder_btn_ref = NodeRef::<Button>::new();

    let toggle_search = move |_| {
        let new_state = !is_search_open.get_untracked();
        set_is_search_open.set(new_state);
        if new_state {
            request_animation_frame(move || {
                if let Some(el) = search_input_ref.get() {
                    let _ = el.focus();
                    el.select();
                }
            });
        }
    };

    // ==========================================
    // GLOBAL KEYBOARD SHORTCUT (Ctrl+F)
    // ==========================================
    crate::utils::window_listener(keydown, move |ev| {
        if (ev.ctrl_key() || ev.meta_key()) && ev.key().to_lowercase() == "f" {
            ev.prevent_default();
            set_is_search_open.set(true);
            request_animation_frame(move || {
                if let Some(el) = search_input_ref.get() {
                    let _ = el.focus();
                    el.select();
                }
            });
        }
    });

    Effect::new(move |_| {
        spawn_local(async move {
            let closure = Closure::wrap(Box::new(move |_: JsValue| {
                let next_tab = Tab::switch_from(&signals.config.active_tab.get_untracked());

                signals
                    .config
                    .set_active_tab
                    .set(next_tab.label().to_string());
                signals.chat.set_unread_count.set(0);

                let filters = signals.config.custom_tab_filters.get_untracked();
                signals
                    .chat
                    .set_unread_counts
                    .update(|counts| next_tab.clear_unread(counts, &filters));

                signals.chat.set_is_at_bottom.set(true);
                actions.save_config.dispatch(());
            }) as Box<dyn FnMut(JsValue)>);

            let _ = crate::tauri_bridge::listen(event::GLOBAL_TAB_SWITCH, &closure).await;
            closure.forget();
        });
    });

    // ==========================================
    // CLICK-OUTSIDE TO CLOSE LISTENERS
    // ==========================================
    crate::utils::window_listener(click, move |ev| {
        let target = event_target::<Node>(&ev);

        // 1. Close Right-Click Menu
        if context_menu_open.get_untracked().is_some() {
            set_context_menu_open.set(None);
        }

        // 2. Close Search
        if is_search_open.get_untracked() {
            let container = search_container_ref.get();
            let btn = search_btn_ref.get();
            let clicked_inside = container
                .map(|c| c.contains(Some(&target)))
                .unwrap_or(false);
            let clicked_btn = btn.map(|b| b.contains(Some(&target))).unwrap_or(false);
            if !clicked_inside && !clicked_btn {
                set_is_search_open.set(false);
            }
        }

        // 3. Close Controls
        if is_controls_open.get_untracked() {
            let container = controls_container_ref.get();
            let btn = folder_btn_ref.get();
            let clicked_inside = container
                .map(|c| c.contains(Some(&target)))
                .unwrap_or(false);
            let clicked_btn = btn.map(|b| b.contains(Some(&target))).unwrap_or(false);
            if !clicked_inside && !clicked_btn {
                set_is_controls_open.set(false);
            }
        }
    });

    view! {
        <nav
            class=move || format!("@container relative z-50 flex flex-nowrap items-center justify-between gap-x-2 px-2 py-1.5 border-b border-base-content/5 min-h-[44px] select-none transition-all duration-300 overflow-visible {}", if signals.config.compact_mode.get() { "!absolute top-0 inset-x-0 !h-8 !min-h-0 !py-0 !pr-0 opacity-0 hover:opacity-100 focus-within:opacity-100 bg-base-300/95 backdrop-blur-md shadow-lg transition-opacity duration-200" } else { nav_bar_bg(signals.config.overlay_opacity.get()) })
            data-tauri-drag-region
        >
            // --- LEFT: DaisyUI Tabs ---
            <div class="flex items-center gap-0.5 bg-base-content/5 p-0.5 rounded-lg min-w-0">
                {move || {
                    Tab::nav(signals.config.debug_mode.get()).into_iter().map(|tab| {
                        let full = tab.label();
                        let db_key = tab.key();
                                                let has_archive_setting = tab.has_archive_setting();
                        let db_key_click = db_key.to_string();
                        let db_key_drop = db_key.to_string();
                        let is_active = move || signals.config.active_tab.get() == full;

                        let unread = Memo::new(move |_| match tab {
                            Tab::Channel(_) | Tab::System => {
                                *signals.chat.unread_counts.get().get(db_key).unwrap_or(&0)
                            }
                            Tab::All | Tab::Custom => 0,
                        });

                                                let dot = tab.dot_class();

                        view! {
                            // REMOVED dropdown classes, replaced with standard relative flex
                            <div class="relative flex items-center h-full">

                                // 1. THE TAB BUTTON
                                <button
                                    title=full
                                    class=move || format!(
                                        "flex items-center gap-1.5 h-7 px-2.5 rounded-md text-xs whitespace-nowrap transition-colors {}",
                                        if is_active() { "bg-base-100 shadow-sm font-bold text-base-content" } else { "font-medium text-base-content/60 hover:text-base-content hover:bg-base-content/5" }
                                    )
                                    on:click=move |_| {
                                        signals.config.set_active_tab.set(full.to_string());
                                        signals.chat.set_unread_count.set(0);
                                        let filters = signals.config.custom_tab_filters.get_untracked();
                                        signals.chat.set_unread_counts.update(|counts| tab.clear_unread(counts, &filters));
                                        signals.chat.set_is_at_bottom.set(true);
                                        signals.chat.set_is_system_at_bottom.set(true);
                                        actions.save_config.dispatch(());
                                    }
                                    on:contextmenu=move |ev| {
                                        ev.prevent_default();
                                        if !matches!(tab, Tab::System | Tab::All) {
                                            set_context_menu_open.set(Some(db_key_click.clone()));
                                        }
                                    }
                                >
                                    <span class=format!("size-2 rounded-full shrink-0 {dot}")></span>
                                    <span class=move || tab_label_class(is_active(), signals.config.compact_mode.get())>{full}</span>
                                    <Show when={move || unread.get() > 0}>
                                        <span class="min-w-4 h-4 px-1 rounded-full bg-error text-error-content text-[10px] font-bold leading-4 text-center">
                                            {move || if unread.get() > 99 { "99+".to_string() } else { unread.get().to_string() }}
                                        </span>
                                    </Show>
                                </button>

                                // 2. THE SETTINGS DROPDOWN MENU
                                <Show when=move || context_menu_open.get() == Some(db_key_drop.clone())>
                                    <ul
                                        class="absolute top-full left-0 z-[100] menu p-3 shadow-2xl bg-base-300 rounded-box w-64 border border-base-content/10 mt-1 space-y-3 cursor-default animate-in fade-in slide-in-from-top-2 duration-100"
                                        on:click=move |ev| ev.stop_propagation() // Prevent clicks from closing the menu
                                    >
                                        <h3 class="text-[11px] font-black opacity-50 border-b border-base-content/10 pb-1.5 uppercase tracking-wider">
                                            {full} " 채널 설정"
                                        </h3>

                                        // --- ONLY FOR CUSTOM TAB: Channel filter selection ---
                                        <Show when=move || tab == Tab::Custom>
                                            <div class="space-y-1 mb-2">
                                                <span class="text-[10px] font-bold text-success">"표시할 채널 선택:"</span>
                                                {Channel::ALL.into_iter().map(|channel| {
                                                    let ch = channel.as_str().to_string();
                                                    let ch_clone = ch.clone();
                                                    view! {
                                                        <label class="label cursor-pointer flex justify-between px-1.5 py-0 hover:bg-base-content/10 rounded">
                                                            <span class="label-text text-[10px] font-bold">{channel.as_str()}</span>
                                                            <input type="checkbox" class="checkbox checkbox-xs checkbox-success"
                                                                checked=move || signals.config.custom_tab_filters.get().contains(&ch_clone)
                                                                on:change=move |ev| {
                                                                    let checked = event_target_checked(&ev);
                                                                    signals.config.set_custom_tab_filters.update(|f| {
                                                                        if checked { f.push(ch.clone()); }
                                                                        else { f.retain(|x| x != &ch); }
                                                                    });
                                                                    actions.save_config.dispatch(());
                                                                }
                                                            />
                                                        </label>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        </Show>

                                        // --- MAX RAM LIMIT INPUT (Hidden for Custom since it aggregates dynamically) ---
                                        <Show when=move || tab != Tab::Custom>
                                            <div class="flex items-center justify-between">
                                                <span class="text-xs font-bold text-base-content/80">"최대 메시지 유지:"</span>
                                                <input type="number" class="input input-xs input-bordered w-16 text-right font-mono bg-base-200 focus:border-success"
                                                    prop:value=move || signals.config.tab_limits.get().get(db_key).copied().unwrap_or_else(|| default_tab_limit(db_key)).to_string()
                                                    on:change=move |ev| {
                                                        let val = event_target_value(&ev).parse::<usize>().unwrap_or_else(|_| default_tab_limit(db_key));
                                                        signals.config.set_tab_limits.update(|map| { map.insert(db_key.to_string(), val); });
                                                        actions.save_config.dispatch(());
                                                    }
                                                />
                                            </div>
                                        </Show>

                                        // --- DISK LOGGING SAVE TOGGLE ---
                                        <Show when=move || has_archive_setting>
                                            <div class="form-control mt-1 pt-2 border-t border-base-content/5">
                                                <label class="label cursor-pointer p-0 hover:bg-transparent">
                                                    <span class="label-text text-xs font-bold text-success">"디스크 자동 저장"</span>
                                                    <input type="checkbox" class="checkbox checkbox-xs checkbox-success"
                                                        prop:checked=move || !signals.config.archive_ignored_channels.get().contains(&db_key.to_string())
                                                        on:change=move |ev| {
                                                            let is_checked = event_target_checked(&ev);
                                                            signals.config.set_archive_ignored_channels.update(|list| {
                                                                if is_checked {
                                                                    // Enable saving = Remove from the ignored list
                                                                    list.retain(|c| c != db_key);
                                                                } else {
                                                                    // Disable saving = Add to the ignored list
                                                                    if !list.contains(&db_key.to_string()) { list.push(db_key.to_string()); }
                                                                }
                                                            });
                                                            actions.save_config.dispatch(());
                                                        }
                                                    />
                                                </label>
                                            </div>
                                        </Show>
                                    </ul>
                                </Show>
                            </div>
                        }
                    }).collect_view()
                }}
            </div>

            // ==========================================
            // CENTER: GLOBAL SEARCH PALETTE
            // ==========================================
            <div
                node_ref=search_container_ref
                class=move || format!(
                    "absolute left-1/2 -translate-x-1/2 top-full mt-2 p-1.5 bg-base-300 border border-base-content/10 rounded-lg shadow-2xl z-50 transition-all duration-200 origin-top {}",
                    if is_search_open.get() { "opacity-100 scale-100 pointer-events-auto" } else { "opacity-0 scale-95 pointer-events-none" }
                )
            >
                <div class="flex items-center gap-1">
                    <input type="text" placeholder="대화 검색 (Ctrl+F)..."
                        node_ref=search_input_ref
                        class="input input-xs input-bordered w-64 bg-base-200 text-xs focus:outline-none focus:border-success"
                        prop:value=move || signals.chat.search_term.get()
                        on:input=move |ev| signals.chat.set_search_term.set(event_target_value(&ev))
                        on:keydown=move |ev| {
                            if ev.key() == "Escape" {
                                set_is_search_open.set(false);
                            }
                        }
                    />
                    <button class="btn btn-ghost btn-xs btn-circle text-base-content/50 hover:text-error"
                        on:click=move |_| {
                            signals.chat.set_search_term.set("".to_string());
                            set_is_search_open.set(false);
                        }>
                        "✕"
                    </button>
                </div>
            </div>

            // --- RIGHT: search, cheat sheet, favorites, pin, then the folded tools
            //     (normal mode) or expand + close (compact mode). Every button is a
            //     title-bar sized `NavButton`, so expand sits where the title bar's
            //     compact button is. ---
            <div class="relative z-10 flex items-center h-full ml-auto shrink-0" data-tauri-no-drag>
                <NavButton title="대화 검색 (Ctrl+F)" button_ref=search_btn_ref
                    active=Signal::derive(move || !signals.chat.search_term.get().is_empty())
                    on_click=toggle_search>
                    {icon(icons::SEARCH, "size-4")}
                </NavButton>
                <NavButton title="직업 · 던전 이름 (일본어 / 한국어)" on_click=open_cheatsheet>
                    {icon(icons::BOOK, "size-4")}
                </NavButton>
                <NavButton title="자주 쓰는 메시지" on_click=open_favorites>
                    {icon(icons::STAR, "size-4")}
                </NavButton>
                <NavButton title="항상 위에 표시" on_click=toggle_pin
                    active=Signal::derive(move || signals.config.always_on_top.get())>
                    <span class=move || if signals.config.always_on_top.get() { "block" } else { "block rotate-45 opacity-60" }>{icon(icons::PIN, "size-4")}</span>
                </NavButton>

                <Show
                    when=move || signals.config.compact_mode.get()
                    fallback=move || view! {
                        // The tools are always folded behind one button: they open
                        // below the bar, so they never cover the tabs.
                        <div class="relative h-full"
                            on:mouseenter=move |_| set_is_controls_open.set(true)
                            on:mouseleave=move |_| set_is_controls_open.set(false)
                        >
                            <NavButton title="도구" button_ref=folder_btn_ref
                                active=is_controls_open
                                on_click=move |_| set_is_controls_open.set(true)>
                                {icon(icons::MORE, "size-4")}
                            </NavButton>

                            <div
                                node_ref=controls_container_ref
                                class=move || if is_controls_open.get() {
                                    "absolute right-0 top-full pt-1 z-[55]"
                                } else {
                                    "hidden"
                                }
                            >
                                <ToolsMenu signals=signals actions=actions />
                            </div>
                        </div>
                    }
                >
                    <NavButton title="컴팩트 모드 끄기"
                        on_click=move |_| {
                            signals.config.set_compact_mode.set(false);
                            actions.save_config.dispatch(());
                        }>
                        {icon(icons::EXPAND, "size-4")}
                    </NavButton>
                    <button class=WINDOW_BUTTON_CLOSE title="닫기"
                        on:click=move |_| { spawn_local(async { let _ = call(&CloseWindow {}).await; }); }>
                        {icon(icons::CLOSE, "size-4")}
                    </button>
                </Show>
            </div>
        </nav>
    }
}

/// The folded tools -- opacity, clear history, settings -- under the bar's `…` button.
#[component]
fn ToolsMenu(signals: AppSignals, actions: AppActions) -> impl IntoView {
    view! {
        <div class="flex items-center gap-1 bg-base-300 p-1 rounded-lg shadow-2xl border border-white/10 animate-in fade-in duration-150">
            <div class="relative group flex items-center justify-center">
                <div class="tooltip tooltip-bottom" data-tip="Background Opacity">
                    <button class="btn btn-ghost btn-sm btn-square text-base-content/60 hover:text-base-content">
                        {icon(icons::CONTRAST, "size-4")}
                    </button>
                </div>
                <div class="absolute top-full right-1/2 translate-x-1/2 pt-1.5 z-50 opacity-0 pointer-events-none group-hover:opacity-100 group-hover:pointer-events-auto transition-all duration-200">
                    <div class="bg-base-300 border border-base-content/10 rounded-lg shadow-xl p-3 w-32 flex flex-col gap-2 items-center cursor-default">
                        <span class="text-[9px] font-black text-success uppercase tracking-widest opacity-80">
                            {move || format!("투명도: {:.0}%", signals.config.overlay_opacity.get() * 100.0)}
                        </span>
                        <input type="range" min="0.0" max="1.0" step="0.05"
                            class="range range-xs range-success w-full"
                            prop:value=move || signals.config.overlay_opacity.get().to_string()
                            on:input=move |ev| {
                                let val = event_target_value(&ev).parse::<f32>().unwrap_or(0.85);
                                signals.config.set_overlay_opacity.set(val);
                            }
                            on:change=move |ev| {
                                let val = event_target_value(&ev).parse::<f32>().unwrap_or(0.85);
                                signals.config.set_overlay_opacity.set(val);
                                actions.save_config.dispatch(());
                            }
                        />
                    </div>
                </div>
            </div>

            <div class="tooltip tooltip-bottom" data-tip="Clear History">
                <button class="btn btn-ghost btn-sm btn-square text-base-content/60 hover:text-base-content hover:!text-error"
                    on:click=move |_| { actions.clear_history.dispatch(()); }>
                    {icon(icons::TRASH, "size-4")}
                </button>
            </div>

            <div class="tooltip tooltip-bottom" data-tip="Settings">
                <button class="btn btn-ghost btn-sm btn-square text-base-content/60 hover:text-base-content" aria-label="Settings" on:click=move |_| signals.ui.set_show_settings.set(true)>
                    {icon(icons::GEAR, "size-4")}
                </button>
            </div>
        </div>
    }
}
