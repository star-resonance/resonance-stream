//! The page of a popup window (`open_popup`): a title bar like the main
//! window's, then one tool, on a solid background with the app's theme.

use crate::components::icons::{self, icon};
use crate::components::{CheatSheetWindow, FavoritesWindow};
use crate::favorites_sync;
use crate::popup_view::escape_closes_popup;
use crate::readability::{WINDOW_BUTTON, WINDOW_BUTTON_CLOSE};
use crate::store::AppSignals;
use crate::tauri_bridge::{call, listen};
use crate::ui_types::Theme;
use leptos::ev::keydown;
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::{event, CloseWindow, LoadConfig, MinimizeWindow};
use resonance_types::PopupKind;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

fn apply_theme(theme: Theme) {
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = root.set_attribute("data-theme", theme.as_str());
    }
}

/// This window's own copy of the settings, from the saved config: the theme is
/// the main window's, and the favorites window checks a shortcut against the
/// tab-switch keys. Only the favorites are ever saved from here
/// (`favorites_sync::save`), never the whole config.
async fn load_settings(signals: AppSignals) {
    if let Ok(config) = call(&LoadConfig {}).await {
        apply_theme(config.theme);
        signals.config.apply(config);
    }
}

/// The popup's title bar: the title and the version on the left, minimize and
/// close on the right, drawn like the main window's (the window itself has no
/// native one). Close hides the window (the backend), so it opens again at once.
#[component]
fn PopupTitleBar(title: &'static str) -> impl IntoView {
    view! {
        <div class="relative z-[60] flex items-center h-8 pl-3 shrink-0 border-b border-base-content/5 bg-base-300 select-none" data-tauri-drag-region>
            <div class="flex items-baseline gap-2 min-w-0 flex-1 pointer-events-none">
                <span class="text-[11px] font-bold text-base-content/80 truncate">{title}</span>
                <span class="text-[10px] text-base-content/40 truncate">{concat!("Resonance Stream v", env!("CARGO_PKG_VERSION"))}</span>
            </div>
            <div class="flex h-8 ml-2 no-drag">
                <button class=WINDOW_BUTTON title="최소화"
                    on:click=move |_| { spawn_local(async { let _ = call(&MinimizeWindow {}).await; }); }>
                    {icon(icons::MINUS, "size-3.5")}
                </button>
                <button class=WINDOW_BUTTON_CLOSE title="닫기"
                    on:click=move |_| { spawn_local(async { let _ = call(&CloseWindow {}).await; }); }>
                    {icon(icons::CLOSE, "size-3.5")}
                </button>
            </div>
        </div>
    }
}

#[component]
pub fn PopupApp(kind: PopupKind) -> impl IntoView {
    apply_theme(Theme::default());
    let signals = AppSignals::new();
    provide_context(signals);
    spawn_local(async move {
        load_settings(signals).await;
        favorites_sync::listen_for_changes(signals.config).await;
        // A popup kept hidden is shown again with an old copy of the settings.
        let shown = Closure::wrap(Box::new(move |_: JsValue| {
            spawn_local(load_settings(signals));
        }) as Box<dyn FnMut(JsValue)>);
        listen(event::POPUP_SHOWN, &shown).await;
        shown.forget();
    });

    let page = view! {
        <div class="h-screen flex flex-col bg-base-100 text-base-content overflow-hidden">
            <PopupTitleBar title=kind.title() />
            // `relative`: the favorites window lays its questions over itself.
            <div class="flex-1 min-h-0 relative">
                {match kind {
                    PopupKind::CheatSheet => view! { <CheatSheetWindow /> }.into_any(),
                    PopupKind::Favorites => view! { <FavoritesWindow /> }.into_any(),
                }}
            </div>
        </div>
    };

    // Esc closes the window -- as the close button does -- unless the page
    // used it (it calls `prevent_default`). Registered after the page is built
    // on purpose: listeners of one target run in the order they were added, so
    // the page's own Esc handling goes first and this one sees what it did.
    window_event_listener(keydown, move |ev| {
        if escape_closes_popup(&ev.key(), ev.is_composing(), ev.default_prevented()) {
            ev.prevent_default();
            spawn_local(async {
                let _ = call(&CloseWindow {}).await;
            });
        }
    });

    page
}
