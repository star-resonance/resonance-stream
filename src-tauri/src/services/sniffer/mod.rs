mod network;
mod raw_capture;
pub(crate) mod replay;

pub use self::network::*;
use self::raw_capture::RawCapture;
pub use self::raw_capture::{open_captures_folder, set_raw_capture};

use crate::{
    inject_system_message, inject_system_message_throttled, store_and_emit, TranslationJob,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::protocol::types::{AppState, SnifferState, SystemLogLevel};
use crossbeam_channel::Sender;
use resonance_core::capture::{ChatPipeline, PipelineAction};
use resonance_core::text::{contains_japanese, convert_to_romaji};
use resonance_core::workers::{
    read_error_backoff, wait_for, watchdog_check, SnifferActivity, WatchdogVerdict,
    WATCHDOG_LIMIT_SECS, WATCHDOG_TICK,
};

/// A started sniffer: dropping it stops the worker threads; `is_alive` says
/// whether the capture thread still runs. A sniffer that never started (no
/// firewall rule) or died (socket error) is not alive, so starting again is
/// not refused as "already active".
pub struct SnifferHandle {
    _stop: Sender<()>,
    alive: Arc<AtomicBool>,
}

impl SnifferHandle {
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Stops the sniffer and waits, up to `wait`, until its capture thread has ended. The thread
    /// drops its socket before it clears the alive flag, so once this returns `true` the socket is
    /// closed and nothing else is capturing: a new sniffer can start without two pipelines seeing
    /// the same packets. `false`: the thread was still running when the time was up.
    pub fn stop_and_wait(self, wait: Duration) -> bool {
        let alive = self.alive.clone();
        drop(self); // closes the stop channel: the capture and watchdog threads end
        wait_for(
            || !alive.load(Ordering::SeqCst),
            wait,
            Duration::from_millis(25),
        )
    }
}

/// Clears the alive flag however the capture thread ends.
struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Unix seconds now, the watchdog's clock.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Records the state (for `get_service_states`) and emits it. The lock is
/// held across the emit so events leave in `seq` order.
pub fn emit_sniffer_state(app: &tauri::AppHandle, state: SnifferState, message: &str) {
    let Some(app_state) = app.try_state::<AppState>() else {
        return;
    };
    let mut states = app_state.service_states.lock();
    let payload = states.set_sniffer(state, message);
    let _ = app.emit(resonance_types::ipc::event::SNIFFER_STATE, payload);
}

/// async: `start_sniffer_worker` asks `netsh` whether the firewall rule exists, which takes
/// hundreds of milliseconds -- not on the main thread, where it froze the window at every
/// start (review W-6). That one check is the only one: a missing rule is reported there
/// (log line, `Error` state, `firewall-missing`), and the worker is not started.
#[tauri::command(async)]
pub fn start_sniffer_command(app: AppHandle, state: State<'_, AppState>) {
    if crate::test_env::no_capture() {
        emit_sniffer_state(&app, SnifferState::Off, "Capture disabled (--no-capture)");
        return;
    }

    if !state.services.start_sniffer(&app) {
        inject_system_message(
            &app,
            SystemLogLevel::Warning,
            "Sniffer",
            "Sniffer restart blocked: already active.",
        );
        emit_sniffer_state(&app, SnifferState::Pending, "Listening for game traffic...");
        state.services.sniffer_activity().mark_inactive();
    }
}

/// Starts a sniffer. `activity` is owned by `Services` for the whole run: it outlives this sniffer.
pub fn start_sniffer_worker(app: AppHandle, activity: Arc<SnifferActivity>) -> SnifferHandle {
    // We use a blank channel just for its lifecycle dropping properties
    let (tx, rx) = crossbeam_channel::unbounded::<()>();
    let alive = Arc::new(AtomicBool::new(false));
    let handle = SnifferHandle {
        _stop: tx,
        alive: alive.clone(),
    };

    if crate::test_env::no_capture() {
        return handle;
    }
    if !check_firewall_rule() {
        inject_system_message(
            &app,
            SystemLogLevel::Warning,
            "Sniffer",
            "Firewall rule missing. Triggering Setup Wizard.",
        );
        emit_sniffer_state(
            &app,
            SnifferState::Error,
            "방화벽 설정 필요 (Setup Required)",
        );

        // Tell the frontend to show the Setup Wizard!
        let _ = app.emit(resonance_types::ipc::event::FIREWALL_MISSING, ());

        // Return immediately without spawning the network thread
        return handle;
    }

    let config = crate::config::current_config(&app);

    activity.start_clock(now_secs());
    spawn_watchdog(app.clone(), rx.clone(), alive.clone(), activity.clone());

    // --- MAIN SNIFFER THREAD ---
    let app_handle = app.clone();
    let rx_main = rx.clone();

    alive.store(true, Ordering::SeqCst);
    thread::spawn(move || {
        let _alive = AliveGuard(alive);
        inject_system_message(
            &app_handle,
            SystemLogLevel::Success,
            "Sniffer",
            "Engine Active",
        );
        emit_sniffer_state(&app_handle, SnifferState::Starting, "Engine Active");
        activity.mark_inactive();

        // Abstracted Network Setup
        let socket = match initialize_network_socket(&app_handle, &config) {
            Some(s) => s,
            None => return,
        };

        inject_system_message(
            &app_handle,
            SystemLogLevel::Success,
            "Sniffer",
            "Raw Socket active. Listening for game traffic...",
        );
        emit_sniffer_state(
            &app_handle,
            SnifferState::Pending,
            "Listening for game traffic...",
        );

        let mut buf = [0u8; 65535];
        let state = app_handle.state::<AppState>();
        let mut pipeline = ChatPipeline::new();
        // History reloaded from disk: the server re-sending it after login is
        // not new chat.
        pipeline.remember(
            &state
                .chat_history
                .lock()
                .values()
                .cloned()
                .collect::<Vec<_>>(),
        );
        // Raw unparsed fields are only useful when reverse-engineering the protocol.
        pipeline.set_keep_unknown_fields(config.debug_mode);
        set_raw_capture(config.raw_capture);
        let mut raw_capture = RawCapture::default();
        let mut read_failures = 0u32;

        loop {
            if let Err(crossbeam_channel::TryRecvError::Disconnected) = rx_main.try_recv() {
                inject_system_message(
                    &app_handle,
                    SystemLogLevel::Info,
                    "Sniffer",
                    "Sniffer thread shutting down.",
                );
                break;
            }

            let uninit_buf = unsafe {
                std::slice::from_raw_parts_mut(
                    buf.as_mut_ptr() as *mut std::mem::MaybeUninit<u8>,
                    buf.len(),
                )
            };
            let n = match socket.recv(uninit_buf) {
                Ok(n) => {
                    read_failures = 0;
                    n
                }
                // The read timeout: nothing arrived, nothing is wrong.
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue
                }
                // A real failure: say so once per streak and back off, so a
                // socket that keeps failing does not spin a core.
                Err(e) => {
                    read_failures += 1;
                    if read_failures == 1 {
                        inject_system_message(
                            &app_handle,
                            SystemLogLevel::Warning,
                            "Sniffer",
                            format!("Socket read failed ({e}); retrying."),
                        );
                    }
                    thread::sleep(read_error_backoff(read_failures));
                    continue;
                }
            };

            // Debug capture of the untouched packet, before anything parses it.
            raw_capture.feed(&app_handle, &buf[..n]);

            // 1. Feed the Pure Pipeline. The raw socket sees every IP packet on
            // the interface, so nothing here may cost more than the parse: the
            // block list is only consulted for actual chat messages.
            let actions = pipeline.feed_network_packet(
                &buf[..n],
                |uid| state.config.read().blocked_users.contains_key(&uid),
                || state.next_pid.fetch_add(1, Ordering::SeqCst),
                || {
                    if activity.record_packet(now_secs()) {
                        emit_sniffer_state(
                            &app_handle,
                            SnifferState::Active,
                            "Listening for game traffic...",
                        );
                    }
                },
            );

            // 2. Dispatch Side Effects
            if !actions.is_empty() {
                dispatch_pipeline_actions(&app_handle, actions);
            }
        }
    });

    handle // Kept in AppState; dropping it stops the sniffer
}

// --- 2. WATCHDOG THREAD ---
fn spawn_watchdog(
    app: AppHandle,
    rx: crossbeam_channel::Receiver<()>,
    capture_alive: Arc<AtomicBool>,
    activity: Arc<SnifferActivity>,
) {
    thread::spawn(move || {
        loop {
            match rx.recv_timeout(WATCHDOG_TICK) {
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }

            let last = activity.last_traffic();
            let now = now_secs();

            let verdict = watchdog_check(last, now, capture_alive.load(Ordering::SeqCst));
            if verdict == WatchdogVerdict::CaptureEnded {
                // The capture is gone: its failure was reported with its own error, and "no game
                // traffic" would only replace it.
                break;
            }
            if verdict == WatchdogVerdict::Stalled {
                // If it was previously active, throw the error state
                // A full-tunnel VPN hides the chat from every adapter: say so.
                let vpn = network::vpn_in_the_way();
                inject_system_message_throttled(
                    &app,
                    SystemLogLevel::Warning,
                    "Sniffer",
                    match &vpn {
                        Some(name) => format!(
                            "Watchdog: No game traffic for {WATCHDOG_LIMIT_SECS}s. The default route runs through a VPN adapter ({name}); \
                             turn the VPN off or exclude the game from it."
                        ),
                        None => format!("Watchdog: No game traffic for {WATCHDOG_LIMIT_SECS}s."),
                    },
                );

                // Emitting "Error" changes the TitleBar badge to Red so the user can click it!
                emit_sniffer_state(
                    &app,
                    SnifferState::Error,
                    &match &vpn {
                        Some(name) => format!(
                            "게임 트래픽 감지 안됨 (VPN 사용 중: {name} - VPN을 끄거나 게임을 터널에서 제외하세요)"
                        ),
                        None => "게임 트래픽 감지 안됨 (클릭하여 어댑터 복구)".to_string(),
                    },
                );
                // Not active, and the clock starts over: another WATCHDOG_LIMIT_SECS before
                // speaking again.
                activity.stall(now_secs());
            }
        }
    });
}

// --- 4. SIDE EFFECT DISPATCHER ---
fn dispatch_pipeline_actions(app: &AppHandle, actions: Vec<PipelineAction>) {
    let state = app.state::<AppState>();
    let use_translation = state.config.read().use_translation;

    for action in actions {
        match action {
            PipelineAction::UpdateBlockedMessage(chat) => {
                let mut history = state.chat_history.lock();
                if let Some(existing_msg) = history.get_mut(chat.pid) {
                    if !existing_msg.is_blocked {
                        existing_msg.is_blocked = true;
                        let _ = app.emit(
                            resonance_types::ipc::event::CHAT_MESSAGE_UPDATE,
                            existing_msg.clone(),
                        );
                    }
                }
            }
            PipelineAction::EmitNewMessage(mut chat) => {
                // Apply Romaji Swap
                if contains_japanese(&chat.nickname) {
                    let mut nick_cache = state.nickname_cache.lock();
                    chat.nickname_romaji = Some(
                        nick_cache
                            .entry(chat.nickname.clone())
                            .or_insert_with(|| convert_to_romaji(&chat.nickname))
                            .clone(),
                    );
                }

                // Dispatch Side Effects. (A duplicate never gets here: the pipeline
                // dropped it, so it is neither translated nor archived.)
                store_and_emit(app, chat.clone());

                // Every Japanese message is owed a translation, even with the
                // translator off or still starting: its next start catches up.
                if contains_japanese(&chat.message) {
                    state.translation_ledger.lock().record(chat.pid);
                }

                // Every message is in the chat log from the moment it arrives; what
                // the translator does later only adds to the training-pair file and a
                // newer chat-log line with the translation.
                crate::services::translator::archive_arrival(app, &chat);
                let translator = state.services.translator_sender();
                match translator.as_ref() {
                    Some(tx) if use_translation && contains_japanese(&chat.message) => {
                        let _ = tx.send(TranslationJob::new(chat));
                    }
                    _ => crate::services::translator::archive_untranslated(app, &chat),
                }
            }
        }
    }
}

#[tauri::command(async)]
pub fn block_user_command(uid: u64, nickname: String, app: tauri::AppHandle) {
    block_user(&app, uid, nickname);
}

/// Puts a sender on the block list (`nickname` is only its label there) and flags their stored rows.
pub fn block_user(app: &AppHandle, uid: u64, nickname: String) {
    let state = app.state::<AppState>();
    // 1. The block list is the config's (the capture reads it from there); this puts it in use and on disk.
    crate::config::modify_config(app, &state, |config| {
        config.blocked_users.insert(uid, nickname);
    });

    // 2. Retroactively scrub existing messages in the UI
    scrub_rows(app, &state, uid, true);
}

#[tauri::command(async)]
pub fn unblock_user_command(uid: u64, app: tauri::AppHandle) {
    unblock_user(&app, uid);
}

/// Takes a sender off the block list and clears the flag on their stored rows.
pub fn unblock_user(app: &AppHandle, uid: u64) {
    let state = app.state::<AppState>();
    // 1. Remove from the config: in use and on disk
    crate::config::modify_config(app, &state, |config| {
        config.blocked_users.remove(&uid);
    });

    // 2. Retroactively un-scrub existing messages in the UI
    scrub_rows(app, &state, uid, false);
}

/// Flags (or clears) every stored row of `uid` and tells the windows. The history lock is held only while the
/// rows are changed, not while each one is emitted: the capture thread needs it to store the next line.
fn scrub_rows(app: &AppHandle, state: &AppState, uid: u64, blocked: bool) {
    let changed = {
        let mut history = state.chat_history.lock();
        resonance_core::history::set_blocked_for_uid(history.values_mut(), uid, blocked)
    };
    for message in changed {
        let _ = app.emit(resonance_types::ipc::event::CHAT_MESSAGE_UPDATE, message);
    }
}

#[tauri::command]
pub fn restart_sniffer_command(app: tauri::AppHandle) {
    // On its own thread (inside `Services`): the wait for the old capture would otherwise freeze the
    // window, since synchronous commands run on the main thread.
    crate::services::owner::Services::restart_sniffer_in_background(&app);
}
