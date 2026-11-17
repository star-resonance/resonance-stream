//! Chat sending through the in-game DLL (`resonance_ingame.dll`): the `send_chat`
//! command, and the setting's load/unload. The logic is in
//! `resonance_core::ingame_send`; the Win32 calls are in `win/`.

mod pin;
#[cfg(windows)]
mod win;

use crate::protocol::types::SystemLogLevel;
use crate::services::downloader::{cancel_module, ensure_module, module_running};
use crate::{inject_system_message, AppState};
use resonance_core::capture::ReturnEvent;
#[cfg(windows)]
use resonance_core::game_clients::client_views;
use resonance_core::game_clients::ClientFact;
use resonance_core::ingame_fetch::{dll_to_load, FetchError, SUCCESS_LINE};
#[cfg(windows)]
use resonance_core::ingame_send::{pick_target, Resident};
use resonance_core::ingame_send::{send_chat_flow, SendBackend, SendError};
#[cfg(windows)]
use resonance_core::ingame_sessions::Sessions;
#[cfg(windows)]
use resonance_core::send_hub::endpoints_of;
use resonance_core::send_hub::VerdictHub;
use resonance_core::send_text::ClientSpacing;
use resonance_types::ipc::{GameClient, SendVerdict};
use resonance_types::SendChatChannel;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use tauri::{AppHandle, State};

/// Spacing between sends, one window per client. `std` mutexes: the flow keeps a client's
/// guard across the send.
static SPACING: LazyLock<ClientSpacing> = LazyLock::new(ClientSpacing::default);

/// Where the sniffer's Returns meet `send_chat`: its wait for the verdict and the server's
/// cooldown per client and channel.
static VERDICT: LazyLock<VerdictHub> = LazyLock::new(VerdictHub::default);

/// The sniffer hands over each Return it framed; one for a connection no send has used is
/// ignored.
pub(crate) fn on_return(ev: &ReturnEvent) {
    VERDICT.on_return(ev);
}

/// The sniffer's latest roster: each connection and its character's nickname.
static CLIENTS: parking_lot::Mutex<Vec<ClientFact>> = parking_lot::Mutex::new(Vec::new());

/// The sniffer stores the roster when it changes (an empty list when it stops).
pub(crate) fn set_clients(facts: Vec<ClientFact>) {
    *CLIENTS.lock() = facts;
}

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

/// Did the last download of the module fail? Then a send says the file is missing (the setting
/// toggle tries again) instead of starting another download.
static LAST_FAILED: AtomicBool = AtomicBool::new(false);

/// The chat line source of the module's download.
const SOURCE: &str = "게임 내 전송";

#[cfg_attr(not(windows), allow(dead_code))]
struct AppBackend {
    app: AppHandle,
}

/// The file the app loads: the debug override, else the module of this version in the data folder.
fn module_path(app: &AppHandle) -> Option<PathBuf> {
    let data = crate::app_dirs::data(app).ok()?;
    Some(dll_to_load(
        pin::dll_path_override(),
        &data,
        env!("CARGO_PKG_VERSION"),
    ))
}

/// Downloads the module in the background unless one runs or the last try failed; the answer for
/// a send that found no file.
fn missing_module(app: &AppHandle) -> SendError {
    if module_running() {
        return SendError::DllDownloading;
    }
    if LAST_FAILED.load(Ordering::SeqCst)
        || pin::INGAME_DLL_SHA256.is_none()
        || pin::dll_path_override().is_some()
    {
        return SendError::DllMissing;
    }
    let app = app.clone();
    std::thread::spawn(move || fetch_module(&app));
    SendError::DllDownloading
}

/// Downloads the module if needed and says so in the chat: the success line after a download,
/// one failure line otherwise (nothing for a cancel). `true`: the file is there.
fn fetch_module(app: &AppHandle) -> bool {
    let Some(pin) = pin::INGAME_DLL_SHA256 else {
        return true;
    };
    if pin::dll_path_override().is_some() {
        return true;
    }
    match ensure_module(app, pin, || WANTED.load(Ordering::SeqCst)) {
        Ok(downloaded) => {
            LAST_FAILED.store(false, Ordering::SeqCst);
            if downloaded && WANTED.load(Ordering::SeqCst) {
                inject_system_message(app, SystemLogLevel::Success, SOURCE, SUCCESS_LINE);
            }
            true
        }
        Err(e) => {
            if e != FetchError::Cancelled {
                LAST_FAILED.store(true, Ordering::SeqCst);
            }
            if let Some(line) = e.chat_line() {
                inject_system_message(app, SystemLogLevel::Warning, SOURCE, line);
            }
            false
        }
    }
}

#[cfg(windows)]
impl SendBackend for AppBackend {
    fn send(
        &mut self,
        target: Option<u32>,
        channel: SendChatChannel,
        text: &str,
    ) -> Result<Option<u32>, SendError> {
        let clients = win::find_clients()?;
        let pid = pick_target(&clients, target)?;
        let mut sessions = SESSIONS.lock();
        // A session of a game that has closed goes first (closing its pipe).
        sessions.retain_live(&clients);
        VERDICT.forget_clients(&clients);
        let session = sessions.get_or_connect(pid, || {
            let dll = module_path(&self.app).ok_or(SendError::DllMissing)?;
            if !dll.is_file() {
                return Err(missing_module(&self.app));
            }
            win::Session::connect(pid, &mut RESIDENT.lock(), &dll)
        })?;
        // Before the DLL is called: the Return can come before its reply is read.
        VERDICT.learn_route(target, &endpoints_of(&win::tcp::chat_connections(), pid));
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
    ) -> Result<Option<u32>, SendError> {
        Err(SendError::OpenFailed)
    }
}

/// Sends `text` to `channel` in the game `client` (a pid, never shown to the user;
/// none is fine with one client) and answers what the server said (waits up to one second
/// for it). Blocks for seconds (a first send loads the module), so it runs off the main thread.
#[tauri::command(async)]
pub fn send_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: SendChatChannel,
    text: String,
    client: Option<u32>,
) -> Result<SendVerdict, String> {
    let setting_on = state.config.read().ingame_send;
    send_chat_flow(
        setting_on,
        &SPACING,
        &VERDICT,
        &mut AppBackend { app },
        client,
        channel,
        &text,
    )
}

/// The connected game clients, each named by its character (a placeholder while unknown).
/// `target` goes back as `send_chat`'s `client`. Empty when no game is connected (or off
/// Windows); it does not depend on the in-game setting.
#[tauri::command(async)]
pub fn game_clients() -> Vec<GameClient> {
    #[cfg(windows)]
    {
        match win::find_clients() {
            Ok(pids) => client_views(&pids, &win::tcp::chat_connections(), &CLIENTS.lock()),
            Err(e) => {
                log::debug!("no game clients: {e}");
                Vec::new()
            }
        }
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// The setting changed: on fetches the module if needed and loads it now (a game that is not
/// running is not an error here, the first send tries again), off cancels a download and
/// unloads the module. Runs on its own thread.
pub(crate) fn apply_setting(app: &AppHandle, on: bool) {
    WANTED.store(on, Ordering::SeqCst);
    if !on {
        cancel_module();
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if !on {
            shutdown();
            return;
        }
        // A new "on" tries again even after a failure.
        LAST_FAILED.store(false, Ordering::SeqCst);
        start(&app);
    });
}

/// At app start with the setting on: the same as turning it on.
pub(crate) fn prepare(app: &AppHandle, on: bool) {
    if on {
        apply_setting(app, true);
    }
}

/// The on path: the module is made ready, then loaded into the games that run.
fn start(app: &AppHandle) {
    if !fetch_module(app) || !WANTED.load(Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    {
        let Some(dll) = module_path(app) else {
            return;
        };
        let clients = match win::find_clients() {
            Ok(clients) => clients,
            Err(e) => {
                log::info!("in-game module not loaded yet: {e}");
                return;
            }
        };
        let mut sessions = SESSIONS.lock();
        sessions.retain_live(&clients);
        VERDICT.forget_clients(&clients);
        for pid in clients {
            if !WANTED.load(Ordering::SeqCst) {
                break;
            }
            // One client's failure does not stop the others.
            if let Err(e) = sessions.get_or_connect(pid, || {
                win::Session::connect(pid, &mut RESIDENT.lock(), &dll)
            }) {
                log::info!("in-game module not loaded in a game yet: {e}");
            }
        }
    }
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
