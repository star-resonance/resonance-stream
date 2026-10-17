//! The one owner of the background services' lifetime (review R-1, W-10): who is running, and the
//! only code that starts, stops or restarts them. Every path that used to take the slot by hand --
//! start-up, the translator command, a settings change, the model swap -- goes through here, so the
//! rules (an old worker is dropped before a new one starts, a stopped worker is retired) live in one
//! place instead of four slightly different copies.
//!
//! A sniffer restart waits for the old capture thread to end before the new one starts (they used to
//! overlap, each with its own pipeline, so a chat packet in the overlap could be shown twice).

use crate::services::sniffer::{start_sniffer_worker, SnifferHandle};
use crate::services::translator::{self, core::ServerSlot, TranslationJob};
use crate::{inject_system_message, AppState, SystemLogLevel};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use resonance_core::workers::{SnifferActivity, WorkerGenerations, SNIFFER_STOP_WAIT};
use std::sync::Arc;
use std::thread;
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct Services {
    /// The running translator's job queue, or `None`. Dropping it ends the worker.
    translator: Mutex<Option<Sender<TranslationJob>>>,
    /// The running sniffer, or `None`. Held for the whole of a restart, so two starts and restarts
    /// never interleave.
    sniffer: Mutex<Option<SnifferHandle>>,
    /// Which translator worker is current: one that was stopped or replaced while it was still
    /// starting up sees the number move and stays silent.
    generations: Arc<WorkerGenerations>,
    /// The llama-server this app started: its PID and port.
    server: Arc<ServerSlot>,
    /// Game traffic: the last packet and whether "active" was said. For the whole run, not one
    /// sniffer start, so a restart keeps the time of the last packet.
    sniffer_activity: Arc<SnifferActivity>,
}

impl Services {
    /// Starts the translator unless one is running (idempotent). Returns whether it started.
    pub fn start_translator(&self, app: &AppHandle) -> bool {
        let mut slot = self.translator.lock();
        if slot.is_some() {
            return false;
        }
        *slot = Some(translator::start_translator_worker(
            app.clone(),
            crate::get_model_path(app),
            self.generations.clone(),
        ));
        true
    }

    /// Drops the running translator, if any, and starts a fresh one (new settings, a swapped
    /// model). The old worker's queue closes first, so it ends; the new one supersedes it.
    pub fn restart_translator(&self, app: &AppHandle) {
        let mut slot = self.translator.lock();
        *slot = None;
        *slot = Some(translator::start_translator_worker(
            app.clone(),
            crate::get_model_path(app),
            self.generations.clone(),
        ));
    }

    /// Stops the translator: drops its queue and retires every worker still starting up, so none of
    /// them reports on, or relaunches, a server that is no longer theirs. Returns whether one was
    /// running.
    pub fn stop_translator(&self) -> bool {
        let was_running = self.translator.lock().take().is_some();
        self.generations.retire();
        was_running
    }

    /// The llama-server this app started (its PID and port).
    pub fn server(&self) -> &Arc<ServerSlot> {
        &self.server
    }

    /// Whether game traffic is coming in, for the sniffer's threads and its start command.
    pub fn sniffer_activity(&self) -> &Arc<SnifferActivity> {
        &self.sniffer_activity
    }

    /// A handle to the running translator's queue, for the sniffer's per-message dispatch.
    pub fn translator_sender(&self) -> Option<Sender<TranslationJob>> {
        self.translator.lock().clone()
    }

    /// Starts the sniffer unless a live one runs (idempotent). Returns whether it started; `false`
    /// means a capture is already active. A handle that never started (no firewall rule) or died is
    /// not alive and is replaced.
    pub fn start_sniffer(&self, app: &AppHandle) -> bool {
        let mut slot = self.sniffer.lock();
        if slot.as_ref().is_some_and(SnifferHandle::is_alive) {
            return false;
        }
        *slot = Some(start_sniffer_worker(
            app.clone(),
            self.sniffer_activity.clone(),
        ));
        true
    }

    /// Stops the sniffer, if any, and waits (bounded) until its capture thread has ended. Under the
    /// sniffer lock; the caller holds `slot`.
    fn stop_sniffer_in(slot: &mut Option<SnifferHandle>, app: &AppHandle) {
        if let Some(old) = slot.take() {
            if !old.stop_and_wait(SNIFFER_STOP_WAIT) {
                inject_system_message(
                    app,
                    SystemLogLevel::Warning,
                    "Sniffer",
                    format!(
                        "The previous capture did not stop within {} s; going on anyway.",
                        SNIFFER_STOP_WAIT.as_secs()
                    ),
                );
            }
        }
    }

    /// Stops the sniffer and starts a fresh one once the old capture thread has really ended.
    /// Blocks for as long as that takes (a fraction of a second; at most `SNIFFER_STOP_WAIT`), so
    /// call it off the main thread -- see [`Services::restart_sniffer_in_background`].
    pub fn restart_sniffer(&self, app: &AppHandle) {
        let mut slot = self.sniffer.lock();
        Self::stop_sniffer_in(&mut slot, app);
        *slot = Some(start_sniffer_worker(
            app.clone(),
            self.sniffer_activity.clone(),
        ));
    }

    /// Stops the sniffer and leaves it stopped (an adapter change before the setup is done).
    pub fn stop_sniffer(&self, app: &AppHandle) {
        let mut slot = self.sniffer.lock();
        Self::stop_sniffer_in(&mut slot, app);
    }

    /// [`Services::restart_sniffer`] on a thread of its own: for a Tauri command or a settings save,
    /// which must not wait.
    pub fn restart_sniffer_in_background(app: &AppHandle) {
        let app = app.clone();
        thread::spawn(move || app.state::<AppState>().services.restart_sniffer(&app));
    }

    /// [`Services::stop_sniffer`] on a thread of its own.
    pub fn stop_sniffer_in_background(app: &AppHandle) {
        let app = app.clone();
        thread::spawn(move || app.state::<AppState>().services.stop_sniffer(&app));
    }
}
