//! Chat sending through the in-game DLL (`resonance_ingame.dll`): the `send_chat`
//! command, and the setting's load/unload. The logic is in
//! `resonance_core::ingame_send`; the Win32 calls are in `win/`.

mod pin;
#[cfg(windows)]
mod win;

use crate::AppState;
use resonance_core::ingame_send::{send_chat_flow, SendBackend, SendError};
use resonance_core::send_text::SendSpacing;
use resonance_types::SendChatChannel;
use std::sync::{LazyLock, Mutex};
use tauri::State;

/// Spacing between sends. A `std` mutex: the flow keeps its guard across the send.
static SPACING: LazyLock<Mutex<SendSpacing>> = LazyLock::new(|| Mutex::new(SendSpacing::default()));

/// The connection to the game, if one is up.
#[cfg(windows)]
static SESSION: parking_lot::Mutex<Option<win::Session>> = parking_lot::Mutex::new(None);

struct AppBackend;

#[cfg(windows)]
impl SendBackend for AppBackend {
    fn send(&mut self, channel: SendChatChannel, text: &str) -> Result<(), SendError> {
        let mut slot = SESSION.lock();
        if slot.is_none() {
            *slot = Some(win::Session::connect()?);
        }
        let session = slot.as_mut().expect("the session was just set");
        let (result, dropped) = session.send(channel, text);
        if dropped {
            // Closing the pipe makes the DLL unload itself; the next send reconnects.
            *slot = None;
        }
        result
    }
}

#[cfg(not(windows))]
impl SendBackend for AppBackend {
    fn send(&mut self, _channel: SendChatChannel, _text: &str) -> Result<(), SendError> {
        Err(SendError::OpenFailed)
    }
}

/// Sends `text` to `channel` in the game. Blocks for seconds (a first send loads
/// the module), so it runs off the main thread.
#[tauri::command(async)]
pub fn send_chat(
    state: State<'_, AppState>,
    channel: SendChatChannel,
    text: String,
) -> Result<(), String> {
    let setting_on = state.config.read().ingame_send;
    send_chat_flow(setting_on, &SPACING, &mut AppBackend, channel, &text)
}

/// The setting changed: on loads the module now (a game that is not running is not
/// an error here, the first send tries again), off unloads it. Runs on its own thread.
pub(crate) fn apply_setting(on: bool) {
    std::thread::spawn(move || {
        if !on {
            shutdown();
            return;
        }
        #[cfg(windows)]
        {
            let mut slot = SESSION.lock();
            if slot.is_none() {
                match win::Session::connect() {
                    Ok(session) => *slot = Some(session),
                    Err(e) => log::info!("in-game module not loaded yet: {e}"),
                }
            }
        }
    });
}

/// Unloads the module and closes the pipe; safe to call twice and at app exit.
pub(crate) fn shutdown() {
    #[cfg(windows)]
    if let Some(session) = SESSION.lock().take() {
        session.unload();
    }
}
