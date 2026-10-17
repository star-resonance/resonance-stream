//! Backend -> UI events that are also kept as history: system log lines
//! ("system-event") and game chat ("packet-event").

use crate::{AppState, ChatMessage, SystemLogLevel, SystemMessage};
use lazy_static::lazy_static;
use parking_lot::Mutex;
use resonance_core::log_throttle::LogThrottle;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::{Emitter, Manager};

lazy_static! {
    static ref LOG_THROTTLE: Mutex<LogThrottle> = Mutex::new(LogThrottle::default());
}

/// [`inject_system_message`] for a line a loop may write again and again:
/// identical lines are held back for a minute, and the next one says how many
/// were. Not for lines a user action can repeat -- those must always show.
pub fn inject_system_message_throttled<S: AsRef<str>>(
    app: &tauri::AppHandle,
    level: SystemLogLevel,
    source: &str,
    message: S,
) {
    let text = LOG_THROTTLE
        .lock()
        .check(source, message.as_ref(), Instant::now());
    if let Some(text) = text {
        inject_system_message(app, level, source, text);
    }
}

pub fn inject_system_message<S: Into<String>>(
    app: &tauri::AppHandle,
    level: SystemLogLevel,
    source: &str,
    message: S,
) {
    let msg = message.into();

    if let Some(state) = app.try_state::<AppState>() {
        let current_pid = state.next_pid.fetch_add(1, Ordering::SeqCst);

        let log_message = format!("[{}] {}", source, msg);
        match level {
            SystemLogLevel::Info | SystemLogLevel::Success => log::info!("{}", log_message),
            SystemLogLevel::Warning => log::warn!("{}", log_message),
            SystemLogLevel::Error => log::error!("{}", log_message),
            SystemLogLevel::Debug => log::debug!("{}", log_message),
            SystemLogLevel::Trace => log::trace!("{}", log_message),
        }

        let system_message = SystemMessage {
            pid: current_pid,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            level,
            source: source.to_string(),
            message: msg,
        };

        // Store in specialized system storage
        {
            let mut sys_hist = state.system_history.lock();
            if sys_hist.len() >= 200 {
                sys_hist.pop_front();
            }
            sys_hist.push_back(system_message.clone());
        }

        let _ = app.emit(resonance_types::ipc::event::SYSTEM_EVENT, &system_message);
    }
}

/// Stores a chat message in the backend history and emits it ("packet-event"). A duplicate never
/// gets here: the capture pipeline decides that (`resonance_core::capture`, including the same
/// message from a second game client), and the sniffer has already set the romaji nickname.
pub fn store_and_emit(app: &tauri::AppHandle, packet: ChatMessage) {
    if let Some(state) = app.try_state::<AppState>() {
        // Store in HOT Storage (bounded by the largest tab limit, oldest dropped first)
        state.chat_history.lock().push(packet.clone());

        // Emit "packet-event" for Game Chat
        let _ = app.emit(resonance_types::ipc::event::PACKET_EVENT, &packet);
    }
}
