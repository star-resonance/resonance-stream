//! Click-through with a hold key: while click-through is on, the main window
//! takes clicks for as long as the hold key is down. A thread polls the key;
//! the decisions are `resonance_core::click_through`.

use parking_lot::Mutex;
use resonance_core::click_through::{ignores_cursor, virtual_key};
use resonance_types::ClickThroughHoldKey;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// How often the hold key is read: quick enough that the overlay takes a
/// click right after the key goes down.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

#[derive(Default)]
struct Inner {
    /// Click-through as the user set it (settings switch, tray).
    enabled: bool,
    hold_key: ClickThroughHoldKey,
    /// What the main window was last told (`set_ignore_cursor_events`).
    ignoring: bool,
}

/// Managed state: click-through, its hold key, and what the window has now.
#[derive(Default)]
pub struct ClickThroughState(Mutex<Inner>);

impl ClickThroughState {
    /// Records click-through on or off; the caller applies it to the window.
    pub fn set_enabled(&self, enabled: bool) {
        let mut inner = self.0.lock();
        inner.enabled = enabled;
        inner.ignoring = enabled;
    }

    pub fn set_hold_key(&self, key: ClickThroughHoldKey) {
        self.0.lock().hold_key = key;
    }

    /// What the window should be told now, when that differs from what it
    /// was last told (and records it). The lock is let go before the window
    /// is called: a window call waits for the main thread, where the
    /// synchronous commands that also take this lock run.
    fn change(&self, key_down: impl Fn(u16) -> bool) -> Option<bool> {
        let mut inner = self.0.lock();
        let down = virtual_key(inner.hold_key).is_some_and(key_down);
        let want = ignores_cursor(inner.enabled, down);
        (want != inner.ignoring).then(|| {
            inner.ignoring = want;
            want
        })
    }
}

/// Starts the thread that lifts click-through while the hold key is down.
pub fn start_hold_key_poller(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("click-through-hold-key".into())
        .spawn(move || loop {
            std::thread::sleep(POLL_INTERVAL);
            let Some(ignore) = app.state::<ClickThroughState>().change(key_down) else {
                continue;
            };
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_ignore_cursor_events(ignore);
            }
        });
    if let Err(e) = spawned {
        log::warn!("Click-through hold key not watched: {e}");
    }
}

#[cfg(target_os = "windows")]
fn key_down(vk: u16) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // SAFETY: GetAsyncKeyState only reads the key state.
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

#[cfg(not(target_os = "windows"))]
fn key_down(_vk: u16) -> bool {
    false
}

#[tauri::command]
pub fn set_click_through_hold_key(
    state: tauri::State<ClickThroughState>,
    key: ClickThroughHoldKey,
) {
    state.set_hold_key(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_told_only_when_the_key_changes_what_it_should_do() {
        let state = ClickThroughState::default();
        state.set_hold_key(ClickThroughHoldKey::Alt);
        // Off: nothing to change, held or not.
        assert_eq!(state.change(|_| true), None);
        state.set_enabled(true);
        assert_eq!(state.change(|_| false), None);
        // Alt (0x12) down: the window takes clicks, once.
        assert_eq!(state.change(|vk| vk == 0x12), Some(false));
        assert_eq!(state.change(|vk| vk == 0x12), None);
        // Another key does not count; let go: clicks pass through again.
        assert_eq!(state.change(|vk| vk == 0x11), Some(true));
        // No hold key: nothing lifts it.
        state.set_hold_key(ClickThroughHoldKey::NoKey);
        assert_eq!(state.change(|_| true), None);
    }

    #[test]
    fn turning_click_through_off_while_the_key_is_down_leaves_the_window_clickable() {
        let state = ClickThroughState::default();
        state.set_enabled(true);
        assert_eq!(state.change(|_| true), Some(false));
        state.set_enabled(false);
        assert_eq!(state.change(|_| true), None);
        assert_eq!(state.change(|_| false), None);
    }
}
