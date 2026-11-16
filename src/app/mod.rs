mod actions;
mod hydration;
mod setup_flow;

use crate::components::settings::Settings;
use crate::components::title_bar::TitleBar;
use crate::components::{
    AddToDictionaryModal, AppUpdateModal, ChatComposer, ChatContainer, DictionaryModal,
    ModelUpdateModal, NavBar, SetupWizard, Troubleshooter,
};
use crate::config_signals::ConfigSignals;
use crate::hooks::use_tray::{setup_tray_listeners, sync_tray_menu};
use crate::store::AppSignals;
use crate::ui_types::Theme;
use crate::view_signals::UiSignals;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn App() -> impl IntoView {
    // --- STATE SIGNALS ---
    let signals = AppSignals::new();
    let ConfigSignals {
        compact_mode,
        theme,
        overlay_opacity: opacity,
        ..
    } = signals.config;
    let UiSignals {
        active_menu,
        set_active_menu,
        ..
    } = signals.ui;
    let set_current_time = signals.chat.set_current_time;

    provide_context(signals);

    // A click anywhere else closes the open row menu. The sender / message
    // triggers and the menus themselves stop their clicks (chat_row.rs), so
    // only a click "somewhere else" gets here.
    window_event_listener(leptos::ev::click, move |_| {
        if active_menu.get_untracked().is_some() {
            set_active_menu.set(None);
        }
    });

    // --- CONFIG ACTIONS ---
    let actions = actions::create_actions(signals);
    let finalize_setup = setup_flow::finalize_setup(signals, actions.save_config);
    let start_download = setup_flow::start_download(signals, finalize_setup);

    provide_context(actions);

    // --- TRAY ICON LISTENER ---
    setup_tray_listeners(signals, actions);

    // Apply theme to the root element whenever it changes
    Effect::new(move |_| {
        if let Some(window) = web_sys::window() {
            if let Some(doc) = window.document() {
                // 1. Apply theme to <html> (DaisyUI standard) and FORCE transparency
                if let Some(html) = doc.document_element() {
                    let _ = html.set_attribute("data-theme", theme.get().as_str());
                    // This strips DaisyUI's solid background so the Tauri window is clear
                    let _ =
                        html.set_attribute("style", "background-color: transparent !important;");
                }

                // 2. Ensure <body> is also fully transparent
                if let Some(body) = doc.body() {
                    let _ =
                        body.set_attribute("style", "background-color: transparent !important;");
                }
            }
        }
    });

    // --- STARTUP HYDRATION ---
    Effect::new(move |_| {
        spawn_local(async move {
            hydration::hydrate_from_backend(signals).await;
        });
    });

    // This automatically runs on startup, AND anytime either variable is changed from anywhere!
    sync_tray_menu(signals);

    // --- TICKER FOR RELATIVE TIME ---
    // Updates the global current_time signal every 10 seconds
    spawn_local(async move {
        loop {
            set_current_time.set(chrono::Local::now().timestamp_millis() as u64);
            gloo_timers::future::TimeoutFuture::new(10_000).await;
        }
    });

    view! {
        <main id="main-app-container"
            class=move || if compact_mode.get() {
                "chat-app compact flex flex-col h-screen overflow-hidden"
            } else {
                "chat-app flex flex-col h-screen overflow-hidden"
            }
            // Natively binds your opacity signal to the DaisyUI theme background
            // style:background-color=move || {
            style=move || {
                let current_opacity = opacity.get();
                if theme.get() == Theme::Dark {
                    format!("background-color: rgba(18, 18, 18, {}) !important;", current_opacity)
                } else {
                    format!("background-color: rgba(252, 252, 252, {}) !important;", current_opacity)
                }
            }
            // Note: Use `signals.config.overlay_opacity.get()` if your app.rs uses the signals struct instead of local signals.
        >
            <Show when=move || !compact_mode.get()>
                <TitleBar />
            </Show>
            <Show
                when=move || signals.config.init_done.get()
                fallback=move || view! {
                    <SetupWizard
                        finalize=Callback::new(finalize_setup)
                        start_download=Callback::new(start_download)
                    />
                }
            >
                <NavBar />

                <ChatContainer />

                <Show when=move || signals.config.ingame_send.get()>
                    <ChatComposer />
                </Show>
            </Show>


            <AppUpdateModal />

            <ModelUpdateModal />

            <Troubleshooter />

            // Dictionary Modal
            <DictionaryModal />

            // Add a term from a chat message
            <AddToDictionaryModal />

            // Favorite Messages Modal

            // Class / dungeon names, Japanese and Korean

            // Settings Modal
            <Settings />

        </main>
    }
}
