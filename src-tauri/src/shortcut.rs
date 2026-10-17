//! Global keyboard shortcuts: switching chat tabs, and pasting a favorite
//! message into the focused window (the game's chat box).
//!
//! The plugin can only unregister everything at once, so every shortcut is
//! registered in one place, `apply_global_shortcuts`, from `AppState.shortcuts`.
//! Changing one kind re-registers all of them.

use crate::{inject_system_message, AppState, SystemLogLevel};
use parking_lot::Mutex;
use resonance_core::paste::RepeatGuard;
use resonance_types::{FavoriteMessage, TabSwitchModifier};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// Time the game gets to read the clipboard after Ctrl+V, before the
/// clipboard text from before the paste is put back.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(500);

/// Survives re-registration, so saving the list does not reset the window.
static REPEAT_GUARD: LazyLock<Mutex<RepeatGuard>> = LazyLock::new(Default::default);
/// Held for a whole paste: a second paste waits, so it does not save the
/// first one's message as "the clipboard before" and restore that.
static PASTE_LOCK: Mutex<()> = Mutex::new(());

/// What the global shortcuts are bound to.
#[derive(Debug, Clone, Default)]
pub struct GlobalShortcuts {
    pub tab_modifier: TabSwitchModifier,
    pub tab_key: String,
    pub favorites: Vec<FavoriteMessage>,
}

#[tauri::command]
pub fn update_global_tab_shortcut(app: tauri::AppHandle, modifier: String, key: String) {
    {
        let state = app.state::<AppState>();
        let mut shortcuts = state.shortcuts.lock();
        shortcuts.tab_modifier = TabSwitchModifier::from_name(&modifier);
        shortcuts.tab_key = key;
    }
    apply_global_shortcuts(&app);
}

/// The tab-switch shortcut as Tauri reads it ("CommandOrControl+TAB"), or
/// `None` when unset.
fn tab_accelerator(modifier: TabSwitchModifier, key: &str) -> Option<String> {
    if key.trim().is_empty() || key == "None" {
        return None;
    }
    // Tauri expects "CommandOrControl" instead of "Ctrl"
    let tauri_mod = match modifier {
        TabSwitchModifier::Ctrl => "CommandOrControl",
        TabSwitchModifier::Alt => "Alt",
        TabSwitchModifier::Shift => "Shift",
        TabSwitchModifier::NoModifier => "",
    };
    // Tauri expects uppercase letters for standard keys
    let tauri_key = key.to_uppercase();
    Some(if tauri_mod.is_empty() {
        tauri_key
    } else {
        format!("{}+{}", tauri_mod, tauri_key)
    })
}

/// Unregister every shortcut, then register the current set.
pub fn apply_global_shortcuts(app: &AppHandle) {
    let current = app.state::<AppState>().shortcuts.lock().clone();
    let global = app.global_shortcut();
    let _ = global.unregister_all();

    let report = |message: String| {
        inject_system_message(app, SystemLogLevel::Error, "Shortcut", message);
    };
    let mut taken: Vec<Shortcut> = Vec::new();

    if let Some(accel) = tab_accelerator(current.tab_modifier, &current.tab_key) {
        match accel.parse::<Shortcut>() {
            Ok(shortcut) => {
                let registered = global.on_shortcut(shortcut, |app_handle, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        // When the global shortcut is pressed, tell the frontend to switch tabs!
                        let _ = app_handle.emit(resonance_types::ipc::event::GLOBAL_TAB_SWITCH, ());
                    }
                });
                match registered {
                    Ok(()) => taken.push(shortcut),
                    Err(e) => report(format!("Tab switch shortcut {accel} not registered: {e}")),
                }
            }
            Err(_) => report(format!("Failed to parse global shortcut: {}", accel)),
        }
    }

    for favorite in current.favorites {
        if favorite.shortcut.is_empty() {
            continue;
        }
        let accel = favorite.shortcut.clone();
        let Ok(shortcut) = accel.parse::<Shortcut>() else {
            report(format!("Failed to parse favorite shortcut: {accel}"));
            continue;
        };
        if taken.contains(&shortcut) {
            report(format!("Favorite shortcut {accel} is already in use"));
            continue;
        }
        let text = favorite.text;
        let accel_key = accel.clone();
        // On release: the main key is up, so the paste does not mix with it.
        let registered = global.on_shortcut(shortcut, move |app_handle, _shortcut, event| {
            if event.state != ShortcutState::Released {
                return;
            }
            if REPEAT_GUARD.lock().allow(&accel_key, Instant::now()) {
                paste_favorite(app_handle.clone(), text.clone());
            } else {
                log::debug!("Favorite shortcut {accel_key} repeated too soon; ignored");
            }
        });
        match registered {
            Ok(()) => taken.push(shortcut),
            Err(e) => report(format!("Favorite shortcut {accel} not registered: {e}")),
        }
    }
}

/// Put `text` on the clipboard, send Ctrl+V to the focused window, then put
/// back the text that was on the clipboard before (only text can be put
/// back; anything else is left replaced). Off the shortcut handler's thread:
/// it waits for the clipboard and the game.
fn paste_favorite(app: AppHandle, text: String) {
    std::thread::spawn(move || {
        let _one_at_a_time = PASTE_LOCK.lock();
        let previous = app.clipboard().read_text().ok();
        if let Err(e) = app.clipboard().write_text(text) {
            inject_system_message(
                &app,
                SystemLogLevel::Error,
                "Shortcut",
                format!("Clipboard write failed: {e}"),
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(30));
        send_paste_keys();

        if let Some(previous) = previous {
            std::thread::sleep(CLIPBOARD_RESTORE_DELAY);
            if let Err(e) = app.clipboard().write_text(previous) {
                log::warn!("Clipboard not restored after paste: {e}");
            }
        }
    });
}

#[cfg(target_os = "windows")]
fn send_paste_keys() {
    use resonance_core::paste::{paste_keystrokes, MODIFIER_KEYS};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };

    // SAFETY: GetAsyncKeyState only reads the key state.
    let held: Vec<u16> = MODIFIER_KEYS
        .into_iter()
        .filter(|&vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0)
        .collect();

    let inputs: Vec<INPUT> = paste_keystrokes(&held)
        .into_iter()
        .map(|key| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key.vk,
                    wScan: 0,
                    dwFlags: if key.up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
        .collect();

    // SAFETY: `inputs` is a live, correctly sized array of INPUT structs.
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Pasting sends Windows key events; elsewhere the text is only copied.
#[cfg(not(target_os = "windows"))]
fn send_paste_keys() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_accelerator_matches_the_old_format() {
        assert_eq!(
            tab_accelerator(TabSwitchModifier::Ctrl, "Tab").as_deref(),
            Some("CommandOrControl+TAB")
        );
        assert_eq!(
            tab_accelerator(TabSwitchModifier::Alt, "q").as_deref(),
            Some("Alt+Q")
        );
        assert_eq!(
            tab_accelerator(TabSwitchModifier::NoModifier, "F2").as_deref(),
            Some("F2")
        );
        assert_eq!(tab_accelerator(TabSwitchModifier::Ctrl, ""), None);
        assert_eq!(tab_accelerator(TabSwitchModifier::NoModifier, "None"), None);
    }

    #[test]
    fn favorite_accelerators_parse() {
        for accel in ["Ctrl+Shift+Digit1", "Alt+Backquote", "F5", "Ctrl+Numpad3"] {
            assert!(accel.parse::<Shortcut>().is_ok(), "{accel}");
        }
    }
}
