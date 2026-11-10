//! Tray <-> UI wiring: tray menu clicks flip UI state, UI state relabels the tray.

use crate::config_signals::ConfigSignals;
use crate::store::{AppActions, AppSignals};
use crate::tauri_bridge::{call, listen};
use crate::view_signals::UiSignals;
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::{
    event, SetAlwaysOnTop, SetClickThrough, SetClickThroughHoldKey, UpdateTrayMenu,
};
use wasm_bindgen::prelude::*;

/// Listens for the tray's click-through and always-on-top menu items.
pub fn setup_tray_listeners(signals: AppSignals, actions: AppActions) {
    let ConfigSignals {
        always_on_top: is_pinned,
        set_always_on_top: set_is_pinned,
        ..
    } = signals.config;
    let UiSignals {
        click_through,
        set_click_through,
        ..
    } = signals.ui;

    spawn_local(async move {
        // 1. Click-Through Listener (Existing)
        let tray_closure = Closure::wrap(Box::new(move |_: JsValue| {
            let current = click_through.get_untracked();
            set_click_through.set(!current);
            actions.save_config.dispatch(());

            spawn_local(async move {
                let _ = call(&SetClickThrough { enabled: !current }).await;
            });
        }) as Box<dyn FnMut(JsValue)>);

        let _ = listen(event::TRAY_TOGGLE_CLICK_THROUGH, &tray_closure).await;
        tray_closure.forget();

        // 2. Always on Top Listener (NEW)
        let tray_top_closure = Closure::wrap(Box::new(move |_: JsValue| {
            let current = is_pinned.get_untracked();
            set_is_pinned.set(!current); // Flip the signal so the TitleBar icon updates!
            actions.save_config.dispatch(()); // Save to config file

            spawn_local(async move {
                let _ = call(&SetAlwaysOnTop { on_top: !current }).await;
            });
        }) as Box<dyn FnMut(JsValue)>);

        let _ = listen(event::TRAY_TOGGLE_ALWAYS_ON_TOP, &tray_top_closure).await;
        tray_top_closure.forget();
    });
}

/// Keeps the tray menu labels in sync with click-through / always-on-top /
/// the hold key, and the backend's hold key with the setting. Runs on
/// start-up and whenever one of them changes.
pub fn sync_tray_menu(signals: AppSignals) {
    let ConfigSignals {
        always_on_top: is_pinned,
        click_through_hold_key,
        ..
    } = signals.config;
    let UiSignals { click_through, .. } = signals.ui;

    Effect::new(move |_| {
        let ct = click_through.get();
        let aot = is_pinned.get();
        let hold_key = click_through_hold_key.get();

        spawn_local(async move {
            let _ = call(&SetClickThroughHoldKey { key: hold_key }).await;
            let _ = call(&UpdateTrayMenu {
                click_through: ct,
                always_on_top: aot,
                hold_key,
            })
            .await;
        });
    });
}
