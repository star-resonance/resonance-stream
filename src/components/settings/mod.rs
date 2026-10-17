mod appearance;
mod blocked_users;
mod chat;
mod data_dev;
mod keywords;
mod translation;

use self::appearance::AppearanceSection;
use self::blocked_users::BlockedUsersSection;
use self::chat::ChatSection;
use self::data_dev::DataDevSection;
use self::keywords::KeywordSection;
use self::translation::TranslationSection;
use crate::dictionary_status::sync_outcome;
use crate::settings_nav::SettingsCategory;
use crate::store::AppSignals;
use crate::tauri_bridge::invoke;
use crate::ui_types::{NetworkInterface, WindowRect};
use leptos::prelude::*;
use leptos::reactive::spawn_local;
use wasm_bindgen::JsValue;

#[cfg(target_arch = "wasm32")] // only the wasm click handler builds it
#[derive(serde::Serialize)]
struct OpenBrowserArgs {
    url: String,
}

/// The smallest window (logical px) the settings view opens in; a smaller
/// window grows to it while settings is open and shrinks back on close.
const SETTINGS_MIN_WIDTH: f64 = 900.0;
const SETTINGS_MIN_HEIGHT: f64 = 640.0;

/// A sidebar icon: its SVG path (heroicons outline) and badge colour.
fn category_icon(category: SettingsCategory) -> (&'static str, &'static str) {
    match category {
        SettingsCategory::Translation => ("M10.5 21l5.25-11.25L21 21m-9-3h7.5M3 5.621a48.474 48.474 0 016-.371m0 0c1.12 0 2.233.038 3.334.114M9 5.25V3m3.334 2.364C11.176 10.658 7.69 15.08 3 17.502m9.334-12.138c.896.061 1.785.147 2.666.257m-4.589 8.495a18.023 18.023 0 01-3.827-5.802", "bg-info text-info-content"),
        SettingsCategory::Chat => ("M20.25 8.511c.884.284 1.5 1.128 1.5 2.097v4.286c0 1.136-.847 2.1-1.98 2.193-.34.027-.68.052-1.02.072v3.091l-3-3c-1.354 0-2.694-.055-4.02-.163a2.115 2.115 0 01-.825-.242m9.345-8.334a2.126 2.126 0 00-.476-.095 48.64 48.64 0 00-8.048 0c-1.131.094-1.976 1.057-1.976 2.192v4.286c0 .837.46 1.58 1.155 1.951m9.345-8.334V6.637c0-1.621-1.152-3.026-2.76-3.235A48.455 48.455 0 0011.25 3c-2.115 0-4.198.137-6.24.402-1.608.209-2.76 1.614-2.76 3.235v6.226c0 1.621 1.152 3.026 2.76 3.235.577.075 1.157.14 1.74.194V21l4.155-4.155", "bg-success text-success-content"),
        SettingsCategory::Keywords => ("M14.857 17.082a23.848 23.848 0 005.454-1.31A8.967 8.967 0 0118 9.75v-.7V9A6 6 0 006 9v.75a8.967 8.967 0 01-2.312 6.022c1.733.64 3.56 1.085 5.455 1.31m5.714 0a24.255 24.255 0 01-5.714 0m5.714 0a3 3 0 11-5.714 0", "bg-warning text-warning-content"),
        SettingsCategory::Appearance => ("M9 17.25v1.007a3 3 0 01-.879 2.122L7.5 21h9l-.621-.621A3 3 0 0115 18.257V17.25m6-12V15a2.25 2.25 0 01-2.25 2.25H5.25A2.25 2.25 0 013 15V5.25m18 0A2.25 2.25 0 0018.75 3H5.25A2.25 2.25 0 003 5.25m18 0V12a2.25 2.25 0 01-2.25 2.25H5.25A2.25 2.25 0 013 12V5.25", "bg-secondary text-secondary-content"),
        SettingsCategory::Blocked => ("M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636", "bg-error text-error-content"),
        SettingsCategory::Data => ("M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 0v3.75m-16.5-3.75v3.75m16.5 0v3.75C20.25 16.153 16.556 18 12 18s-8.25-1.847-8.25-4.125v-3.75m16.5 0c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125", "bg-neutral text-neutral-content"),
    }
}

/// A category's icon on its coloured rounded badge; `size` is the badge's classes.
fn category_badge(category: SettingsCategory, size: &'static str) -> impl IntoView {
    let (path, colour) = category_icon(category);
    view! {
        <span class=format!("{size} {colour} rounded-lg flex items-center justify-center shrink-0 shadow-sm")>
            <svg class="w-3/5 h-3/5" fill="none" stroke="currentColor" stroke-width="1.8" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" d=path />
            </svg>
        </span>
    }
}

#[component]
pub fn Settings() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");

    let (interfaces, set_interfaces) = signal(Vec::<NetworkInterface>::new());
    let (new_keyword, set_new_keyword) = signal(String::new());
    let (new_emphasis, set_new_emphasis) = signal(String::new());

    // The rect the window had before settings grew it; restored on close.
    let (restore_rect, set_restore_rect) = signal(None::<WindowRect>);

    Effect::new(move |_| {
        if signals.ui.show_settings.get() {
            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&serde_json::json!({
                    "minWidth": SETTINGS_MIN_WIDTH,
                    "minHeight": SETTINGS_MIN_HEIGHT,
                }))
                .unwrap();
                let Ok(res) = invoke("grow_window", args).await else {
                    return;
                };
                let Ok(Some(rect)) = serde_wasm_bindgen::from_value::<Option<WindowRect>>(res)
                else {
                    return;
                };
                if signals.ui.show_settings.get_untracked() {
                    set_restore_rect.set(Some(rect));
                } else {
                    // closed again before the window finished growing
                    restore_window(rect);
                }
            });
            spawn_local(async move {
                if let Ok(res) = invoke("get_network_interfaces", JsValue::NULL).await {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<NetworkInterface>>(res) {
                        set_interfaces.set(list);
                    }
                }
            });
        } else {
            signals.service.set_restart_required.set(false);
            if let Some(rect) = restore_rect.get_untracked() {
                set_restore_rect.set(None);
                restore_window(rect);
            }
        }
    });

    let sync_dict_action = Action::new_local(|_: &()| async move {
        let reason = |e: JsValue| e.as_string().unwrap_or_else(|| format!("{e:?}"));
        // The backend syncs what the signed metadata names; a refused publication is its error.
        let result = invoke("sync_dictionary", JsValue::NULL)
            .await
            .map(|_| ())
            .map_err(reason);
        sync_outcome(result)
    });
    let save_chat_action = Action::new_local(move |_: &()| {
        // 1. Extract the raw chat messages from the signal map
        let logs_to_export: Vec<_> = signals
            .chat
            .chat
            .with_untracked(|store| store.all())
            .into_iter()
            .map(|sig| sig.get_untracked()) // Unpack the RwSignal<ChatMessage>
            .collect();

        // 2. Send them to Tauri
        async move {
            let args = serde_wasm_bindgen::to_value(&serde_json::json!({ "logs": logs_to_export }))
                .unwrap();

            match invoke("export_chat_log", args).await {
                Ok(_) => "저장 완료".to_string(),
                Err(_) => "저장 실패".to_string(),
            }
        }
    });

    let close = move |_| signals.ui.set_show_settings.set(false);
    let category = signals.ui.settings_category;

    view! {
        <Show when=move || signals.ui.show_settings.get()>
            <div class="modal modal-open backdrop-blur-sm transition-all duration-300 z-[20000]">
                <div class="modal-box bg-base-100 border border-base-content/10 p-0 overflow-hidden shadow-2xl animate-in zoom-in duration-200 flex
                            w-[calc(100vw-1.5rem)] max-w-5xl h-[calc(100vh-1.5rem)] max-h-[760px]">

                    // --- SIDEBAR: categories ---
                    <nav class="w-44 sm:w-52 shrink-0 flex flex-col bg-base-200 border-r border-base-content/10">
                        <div class="px-4 pt-4 pb-3">
                            <h2 class="text-sm font-black tracking-widest text-base-content">"SETTINGS"</h2>
                        </div>
                        <ul class="flex-1 overflow-y-auto custom-scrollbar px-2 space-y-0.5">
                            {SettingsCategory::ALL.into_iter().map(|c| {
                                let active = move || category.get() == c;
                                view! {
                                    <li>
                                        <button
                                            class="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left text-sm font-semibold transition-colors"
                                            class=("bg-success", active)
                                            class=("text-success-content", active)
                                            class=("text-base-content/80", move || !active())
                                            class=("hover:bg-base-content/10", move || !active())
                                            on:click=move |_| signals.ui.set_settings_category.set(c)
                                        >
                                            {category_badge(c, "w-6 h-6")}
                                            <span class="truncate">{c.title()}</span>
                                        </button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>

                        // --- GitHub link ---
                        <div class="p-2 border-t border-base-content/10">
                            <button
                                on:click=move |_| {
                                    // Call the Rust backend to open the browser
                                    #[cfg(target_arch = "wasm32")]
                                    spawn_local(async move {
                                        let args = serde_wasm_bindgen::to_value(&OpenBrowserArgs {
                                            url: "https://github.com/enjay27/resonance-stream".to_string(),
                                        }).unwrap();

                                        // Adjust this `invoke` call to match whatever binding
                                        // you use for your other Tauri commands!
                                        let _ = invoke("open_browser", args).await;
                                    });
                                }
                                class="btn btn-ghost btn-xs w-full gap-2 text-base-content/50 hover:text-success transition-all lowercase italic"
                            >
                                <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 16 16"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>
                                {concat!("v", env!("CARGO_PKG_VERSION"))}
                            </button>
                        </div>
                    </nav>

                    // --- PANE: the selected category ---
                    <div class="flex-1 min-w-0 flex flex-col">
                        <div class="flex justify-end px-3 pt-2">
                            <button class="btn btn-ghost btn-xs text-xl" on:click=close>"✕"</button>
                        </div>

                        <div class="flex-1 overflow-y-auto custom-scrollbar p-4 sm:p-6">
                            <div class="max-w-2xl mx-auto space-y-5">
                                // header card, like macOS System Settings
                                <div class="flex flex-col items-center text-center gap-2 bg-base-200 rounded-xl border border-base-content/5 px-4 py-5">
                                    {move || category_badge(category.get(), "w-14 h-14")}
                                    <h3 class="text-lg font-black text-base-content">{move || category.get().title()}</h3>
                                    <p class="text-xs text-base-content/60 max-w-md">{move || category.get().description()}</p>
                                </div>

                                {move || match category.get() {
                                    SettingsCategory::Translation => view! { <TranslationSection /> }.into_any(),
                                    SettingsCategory::Chat => view! { <ChatSection /> }.into_any(),
                                    SettingsCategory::Keywords => view! {
                                        <KeywordSection new_keyword set_new_keyword new_emphasis set_new_emphasis />
                                    }.into_any(),
                                    SettingsCategory::Appearance => view! { <AppearanceSection /> }.into_any(),
                                    SettingsCategory::Blocked => view! { <BlockedUsersSection /> }.into_any(),
                                    SettingsCategory::Data => view! {
                                        <DataDevSection interfaces sync_dict_action save_chat_action />
                                    }.into_any(),
                                }}
                            </div>
                        </div>
                    </div>
                </div>

                // Modal Backdrop to close
                <div class="modal-backdrop bg-black/40" on:click=close></div>
            </div>
        </Show>
    }
}

/// Puts the window back to the rect `grow_window` returned.
fn restore_window(rect: WindowRect) {
    spawn_local(async move {
        let args = serde_wasm_bindgen::to_value(&serde_json::json!({ "rect": rect })).unwrap();
        let _ = invoke("restore_window", args).await;
    });
}
