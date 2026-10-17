use crate::AppState;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use tauri::{AppHandle, Manager};

/// Port tried first for llama-server; another is picked if it is taken.
pub const PREFERRED_SERVER_PORT: u16 = 8080;

/// The llama-server this app started: its PID and the port it was given. Owned by `Services`;
/// a `ServerGuard` keeps a handle so it can clear its own PID when it ends.
#[derive(Debug)]
pub struct ServerSlot {
    /// PID of the llama-server this app started (0: none). Only this process, or the one recorded
    /// in the PID file by a previous run, is ever killed -- never other llama-server.exe instances
    /// the user may be running.
    pid: AtomicU32,
    port: AtomicU16,
}

impl Default for ServerSlot {
    fn default() -> Self {
        Self {
            pid: AtomicU32::new(0),
            port: AtomicU16::new(PREFERRED_SERVER_PORT),
        }
    }
}

impl ServerSlot {
    /// Base URL of the running llama-server (the port is chosen at launch), or the stand-in a test
    /// run named with `--llama-url`.
    pub fn url(&self) -> String {
        if let Some(url) = crate::test_env::llama_url() {
            return url.to_string();
        }
        format!("http://127.0.0.1:{}", self.port.load(Ordering::Relaxed))
    }

    pub(crate) fn set_port(&self, port: u16) {
        self.port.store(port, Ordering::Relaxed);
    }

    pub(crate) fn set_pid(&self, pid: u32) {
        self.pid.store(pid, Ordering::SeqCst);
    }

    /// The recorded PID, now cleared (0: none was recorded).
    pub(crate) fn take_pid(&self) -> u32 {
        self.pid.swap(0, Ordering::SeqCst)
    }

    /// Clears the recorded PID if it is still `pid`: a newer server may already have replaced it.
    pub(crate) fn clear_pid(&self, pid: u32) {
        let _ = self
            .pid
            .compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

/// Base URL of the running llama-server, or the stand-in a test run named with `--llama-url`.
pub fn server_url(app: &AppHandle) -> String {
    app.state::<AppState>().services.server().url()
}
