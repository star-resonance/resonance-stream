//! System tray: menu, its events, and the command that keeps its labels in sync.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};

pub struct TrayMenuState {
    pub click_through: tauri::menu::MenuItem<tauri::Wry>,
    pub always_on_top: tauri::menu::MenuItem<tauri::Wry>,
}

/// Builds the tray icon and menu, and manages `TrayMenuState`.
pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let toggle_i = MenuItem::with_id(
        app,
        "toggle_click_through",
        "클릭 관통 (Click-Through): OFF",
        true,
        None::<&str>,
    )?;
    let top_i = MenuItem::with_id(
        app,
        "toggle_always_on_top",
        "항상 위에 표시 (Always on Top): OFF",
        true,
        None::<&str>,
    )?;
    let show_i = MenuItem::with_id(app, "show", "앱 열기 (Open App)", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "종료 (Quit)", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&top_i, &toggle_i, &show_i, &quit_i])?;

    // Store the items in Tauri's managed state so the command can mutate them later
    app.manage(TrayMenuState {
        click_through: toggle_i.clone(),
        always_on_top: top_i.clone(),
    });

    let _tray = TrayIconBuilder::new()
        .tooltip("Resonance Stream")
        .icon(app.default_window_icon().unwrap().clone()) // Uses icon from tauri.conf.json
        .menu(&menu)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "toggle_always_on_top" => {
                    let _ = app.emit(resonance_types::ipc::event::TRAY_TOGGLE_ALWAYS_ON_TOP, ());
                }
                "toggle_click_through" => {
                    // Tell the frontend to flip the toggle and update the window
                    let _ = app.emit(resonance_types::ipc::event::TRAY_TOGGLE_CLICK_THROUGH, ());
                }
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

#[tauri::command]
pub fn update_tray_menu(
    state: tauri::State<TrayMenuState>,
    click_through: bool,
    always_on_top: bool,
) {
    let ct_text = if click_through {
        "클릭 관통 (Click-Through): ON"
    } else {
        "클릭 관통 (Click-Through): OFF"
    };
    let _ = state.click_through.set_text(ct_text);

    let aot_text = if always_on_top {
        "항상 위에 표시 (Always on Top): ON"
    } else {
        "항상 위에 표시 (Always on Top): OFF"
    };
    let _ = state.always_on_top.set_text(aot_text);
}
