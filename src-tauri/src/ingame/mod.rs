//! Chat sending through the in-game DLL (`resonance_ingame.dll`): the `send_chat`
//! command, and the setting's load/unload. The logic is in
//! `resonance_core::ingame_send`; the Win32 calls are in `win/`.

mod pin;
#[cfg(windows)]
mod win;

use crate::AppState;
#[cfg(windows)]
use resonance_core::ingame_send::{pick_target, Resident};
use resonance_core::ingame_send::{send_chat_flow, SendBackend, SendError};
#[cfg(windows)]
use resonance_core::ingame_sessions::Sessions;
use resonance_core::send_text::SendSpacing;
use resonance_types::SendChatChannel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use tauri::State;

/// Spacing between sends. A `std` mutex: the flow keeps its guard across the send.
static SPACING: LazyLock<Mutex<SendSpacing>> = LazyLock::new(|| Mutex::new(SendSpacing::default()));

/// The connections to the games, one per game pid.
#[cfg(windows)]
static SESSIONS: parking_lot::Mutex<Sessions<win::Session>> =
    parking_lot::Mutex::new(Sessions::new());

/// The games whose module stayed loaded after an unload. Locked after `SESSIONS`,
/// never the other way round.
#[cfg(windows)]
static RESIDENT: parking_lot::Mutex<Resident> = parking_lot::Mutex::new(Resident::new());

/// What the setting last said; checked under the session lock, so a load that
/// was queued by "on" does not run after "off".
static WANTED: AtomicBool = AtomicBool::new(false);

struct AppBackend;

#[cfg(windows)]
impl SendBackend for AppBackend {
    fn send(
        &mut self,
        target: Option<u32>,
        channel: SendChatChannel,
        text: &str,
    ) -> Result<(), SendError> {
        let clients = win::find_clients()?;
        let pid = pick_target(&clients, target)?;
        let mut sessions = SESSIONS.lock();
        // A session of a game that has closed goes first (closing its pipe).
        sessions.retain_live(&clients);
        let session =
            sessions.get_or_connect(pid, || win::Session::connect(pid, &mut RESIDENT.lock()))?;
        let (result, dropped) = session.send(channel, text);
        if dropped {
            // Closing the pipe makes the DLL unload itself; the next send reconnects.
            sessions.drop_pid(pid);
        }
        result
    }
}

#[cfg(not(windows))]
impl SendBackend for AppBackend {
    fn send(
        &mut self,
        _target: Option<u32>,
        _channel: SendChatChannel,
        _text: &str,
    ) -> Result<(), SendError> {
        Err(SendError::OpenFailed)
    }
}

/// Sends `text` to `channel` in the game `client` (a pid, never shown to the user;
/// none is fine with one client). Blocks for seconds (a first send loads
/// the module), so it runs off the main thread.
#[tauri::command(async)]
pub fn send_chat(
    state: State<'_, AppState>,
    channel: SendChatChannel,
    text: String,
    client: Option<u32>,
) -> Result<(), String> {
    let setting_on = state.config.read().ingame_send;
    send_chat_flow(
        setting_on,
        &SPACING,
        &mut AppBackend,
        client,
        channel,
        &text,
    )
}

/// The setting changed: on loads the module now (a game that is not running is not
/// an error here, the first send tries again), off unloads it. Runs on its own thread.
pub(crate) fn apply_setting(on: bool) {
    WANTED.store(on, Ordering::SeqCst);
    std::thread::spawn(move || {
        if !on {
            shutdown();
            return;
        }
        #[cfg(windows)]
        {
            let clients = match win::find_clients() {
                Ok(clients) => clients,
                Err(e) => {
                    log::info!("in-game module not loaded yet: {e}");
                    return;
                }
            };
            let mut sessions = SESSIONS.lock();
            sessions.retain_live(&clients);
            for pid in clients {
                if !WANTED.load(Ordering::SeqCst) {
                    break;
                }
                // One client's failure does not stop the others.
                if let Err(e) = sessions
                    .get_or_connect(pid, || win::Session::connect(pid, &mut RESIDENT.lock()))
                {
                    log::info!("in-game module not loaded in a game yet: {e}");
                }
            }
        }
    });
}

/// Unloads the module from every game and closes the pipes; safe to call twice and
/// at app exit.
pub(crate) fn shutdown() {
    #[cfg(windows)]
    {
        let all = SESSIONS.lock().take_all();
        for (_, session) in all {
            if let Some(pid) = session.unload() {
                RESIDENT.lock().note(pid);
            }
        }
    }
}
