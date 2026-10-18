//! Tauri commands that act on the overlay window itself, and the popup
//! windows that open beside it.

use parking_lot::Mutex;
use resonance_core::window::GrowMemory;
use resonance_types::{is_popup_label, PopupKind};
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_window_state::{AppHandleExt, StateFlags, WindowExt};

#[tauri::command]
pub fn set_always_on_top(window: tauri::Window, on_top: bool) {
    pin(&window, on_top);
}

/// Pins the window over the others (or lets it go), and the popups with it.
pub fn pin(window: &tauri::Window, on_top: bool) {
    // This simple method toggles the window state
    let _ = window.set_always_on_top(on_top);
    // The popups stay with the overlay: pinned over the game when it is.
    for (label, popup) in window.app_handle().webview_windows() {
        if is_popup_label(&label) {
            let _ = popup.set_always_on_top(on_top);
        }
    }
}

/// Creates a popup's window, hidden. Its page starts loading at once, so by
/// the time it is shown it is ready. It has no native title bar (the page
/// draws its own, like the main window's) and is owned by the overlay (above
/// it, closed with it), pinned when the overlay is. The page in it picks its
/// view from the window's label (`PopupKind::label`).
fn create_popup(app: &AppHandle, kind: PopupKind) -> Result<WebviewWindow, String> {
    let main = app
        .get_webview_window("main")
        .ok_or("the main window was not found")?;
    let (width, height) = kind.size();
    let mut builder =
        WebviewWindowBuilder::new(app, kind.label(), WebviewUrl::App("index.html".into()))
            .title(kind.title())
            .inner_size(width, height)
            .min_inner_size(300.0, 300.0)
            .resizable(true)
            .decorations(false)
            .shadow(true)
            .center()
            // Hidden until `show_popup` has put it back where it was left: shown
            // at once it would appear at the default place and jump.
            .visible(false)
            .always_on_top(main.is_always_on_top().unwrap_or(false))
            .parent(&main)
            .map_err(|e| e.to_string())?;
    // `--devtools-port`: the same browser arguments as the main window (WebView2 refuses two sets).
    if let Some(args) = crate::test_env::webview_browser_args() {
        builder = builder.additional_browser_args(&args);
    }
    let window = builder.build().map_err(|e| e.to_string())?;
    // Closing a popup only hides it: opening it again is instant.
    let hider = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hider.hide();
        }
    });
    Ok(window)
}

/// Shows a popup (creating it if it was not made yet), at its saved size and
/// place, in front. A popup that is already open is only brought forward.
pub fn show_popup(app: &AppHandle, kind: PopupKind) -> Result<(), String> {
    let window = match app.get_webview_window(kind.label()) {
        Some(window) => window,
        None => create_popup(app, kind)?,
    };
    let mut restored = false;
    // Without the plugin (`--no-window-state`) there is nothing to restore from.
    if !crate::test_env::no_window_state() && !window.is_visible().unwrap_or(false) {
        // Size and place only: with `VISIBLE` the plugin would show it itself,
        // before we are ready.
        let _ = window.restore_state(StateFlags::SIZE | StateFlags::POSITION);
        restored = true;
    }
    let _ = window.unminimize();
    window.show().map_err(|e| e.to_string())?;
    // The plugin saved the INNER size. Set while the window is hidden, an undecorated window came back about 30 px
    // taller than it was left (the bridge's `PP-place`, real app, 2 runs); set again once shown it is exact, as
    // `apply_rect` is. The place was set before showing, so nothing jumps.
    if restored {
        let _ = window.restore_state(StateFlags::SIZE | StateFlags::POSITION);
    }
    // A popup kept hidden has an old copy of the settings: let its page refresh.
    let _ = app.emit_to(kind.label(), resonance_types::ipc::event::POPUP_SHOWN, ());
    window.set_focus().map_err(|e| e.to_string())
}

/// Opens a tool in a window of its own, so the chat stays visible (see
/// `show_popup`).
///
/// `async` on purpose: creating a window from a synchronous command can
/// deadlock on Windows.
#[tauri::command]
pub async fn open_popup(app: AppHandle, kind: PopupKind) -> Result<(), String> {
    show_popup(&app, kind)
}

/// One window as the bridge's `snapshot-popups` reports it (`null` for a window that does not exist).
fn window_json(app: &AppHandle, label: &str) -> serde_json::Value {
    let Some(webview) = app.get_webview_window(label) else {
        return serde_json::json!({ "exists": false });
    };
    let rect = app
        .get_window(label)
        .and_then(|window| window_rect(&window))
        .and_then(|rect| serde_json::to_value(rect).ok());
    let inner = webview
        .inner_size()
        .ok()
        .map(|size| serde_json::json!({ "width": size.width, "height": size.height }));
    serde_json::json!({
        "exists": true,
        "visible": webview.is_visible().unwrap_or(false),
        "inner": inner,
        "always_on_top": webview.is_always_on_top().unwrap_or(false),
        "rect": rect,
    })
}

/// Each popup's window and the main one as they are now (the bridge's `snapshot-popups`).
pub fn popups_snapshot(app: &AppHandle) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert("main".into(), window_json(app, "main"));
    for kind in PopupKind::ALL {
        out.insert(kind.label().into(), window_json(app, kind.label()));
    }
    let mut labels: Vec<String> = app.webview_windows().into_keys().collect();
    labels.sort();
    out.insert("labels".into(), serde_json::json!(labels));
    serde_json::Value::Object(out)
}

/// What the popup's X does (the bridge's `hide-popup`): a close request, which hides it.
pub fn hide_popup(app: &AppHandle, kind: PopupKind) -> Result<(), String> {
    let window = app
        .get_webview_window(kind.label())
        .ok_or_else(|| format!("{} does not exist", kind.label()))?;
    window.close().map_err(|e| e.to_string())
}

/// Puts a popup at an outer rect, as dragging and resizing it would (the bridge's `place-popup`).
pub fn place_popup(
    app: &AppHandle,
    kind: PopupKind,
    rect: resonance_types::WindowRect,
) -> Result<(), String> {
    let window = app
        .get_window(kind.label())
        .ok_or_else(|| format!("{} does not exist", kind.label()))?;
    apply_rect(&window, rect);
    Ok(())
}

/// Creates every popup's window, hidden, a moment after start-up, so opening
/// one is instant (the cost is the idle web view of each). Later, off the main
/// thread: building a window there can deadlock on Windows.
pub fn prewarm_popups(app: AppHandle) {
    if crate::test_env::no_popups() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        for kind in PopupKind::ALL {
            if app.get_webview_window(kind.label()).is_none() {
                if let Err(e) = create_popup(&app, kind) {
                    log::warn!("popup {} not prepared: {e}", kind.label());
                }
            }
        }
    });
}

/// Closing the overlay ends the app, so the hidden popups (which would keep it
/// alive as windows) go with it.
pub fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    if window.label() == "main" && matches!(event, WindowEvent::CloseRequested { .. }) {
        restore_grown_window(window.app_handle());
        for (label, popup) in window.app_handle().webview_windows() {
            if is_popup_label(&label) {
                let _ = popup.destroy();
            }
        }
    }
}

#[tauri::command]
pub fn minimize_window(window: tauri::Window) {
    let _ = window.minimize();
}

#[tauri::command]
pub fn close_window(window: tauri::Window) {
    let _ = window.close();
}

#[tauri::command]
pub fn set_click_through(window: tauri::Window, enabled: bool) {
    let _ = window.set_ignore_cursor_events(enabled);
}

#[tauri::command]
pub fn grow_window(
    window: tauri::Window,
    min_width: f64,
    min_height: f64,
) -> Option<resonance_types::WindowRect> {
    grow(&window, min_width, min_height)
}

/// Grows the window to at least `min_width` x `min_height` logical pixels,
/// staying on its monitor's work area (the settings view wants more room than
/// the overlay usually has). Returns the rect it replaced, for
/// `restore_window`; `None` when nothing changed.
pub fn grow(
    window: &tauri::Window,
    min_width: f64,
    min_height: f64,
) -> Option<resonance_types::WindowRect> {
    if window.is_maximized().unwrap_or(false) {
        return None;
    }
    let monitor = window.current_monitor().ok().flatten()?;
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();

    let current = resonance_types::WindowRect {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
    };
    let work_area = resonance_types::WindowRect {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width,
        height: area.size.height,
    };
    let grown = resonance_core::window::grow_to_fit(
        current,
        (min_width * scale).round() as u32,
        (min_height * scale).round() as u32,
        work_area,
    )?;
    apply_rect(window, grown);
    GROWN_FROM.lock().remember(current);
    Some(current)
}

/// The window's outer rect in physical pixels (the bridge's `snapshot`).
pub fn window_rect(window: &tauri::Window) -> Option<resonance_types::WindowRect> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(resonance_types::WindowRect {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
    })
}

/// What `grow_window` replaced and `restore_window` has not put back yet.
static GROWN_FROM: Mutex<GrowMemory> = Mutex::new(GrowMemory::new());

/// Puts the main window back to its size from before settings grew it, when the
/// app is closing with settings still open: the window-state plugin saves what
/// the window has at that moment, and the next start would open enlarged.
pub fn restore_grown_window(app: &tauri::AppHandle) {
    let Some(rect) = GROWN_FROM.lock().take() else {
        return;
    };
    let Some(window) = app.get_window("main") else {
        return;
    };
    apply_rect(&window, rect);
    // The plugin's own save at close may already have run; save again, now
    // that the window is back (it re-reads the live size).
    if !crate::test_env::no_window_state() {
        let _ = app.save_window_state(StateFlags::all());
    }
}

/// Puts the window back where `grow_window` found it.
#[tauri::command]
pub fn restore_window(window: tauri::Window, rect: resonance_types::WindowRect) {
    GROWN_FROM.lock().forget();
    apply_rect(&window, rect);
}

/// Puts the window at `rect`, an OUTER rect (what `outer_position` / `outer_size` report and what is
/// remembered): `set_size` takes the inner size, so the frame is taken off first -- otherwise every
/// restore made the window one frame bigger (the K18 runbook run: +22 x +13 px each time).
fn apply_rect(window: &tauri::Window, rect: resonance_types::WindowRect) {
    let (frame_width, frame_height) = match (window.outer_size(), window.inner_size()) {
        (Ok(outer), Ok(inner)) => (
            outer.width.saturating_sub(inner.width),
            outer.height.saturating_sub(inner.height),
        ),
        _ => (0, 0),
    };
    let (width, height) = resonance_core::window::inner_size_for(rect, frame_width, frame_height);
    let _ = window.set_size(tauri::PhysicalSize::new(width, height));
    let _ = window.set_position(tauri::PhysicalPosition::new(rect.x, rect.y));
}
