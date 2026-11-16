//! The Windows-only half of the DLL: IL2CPP binding, the pipe client and the entry points.
//! It cannot be unit-tested (it needs the game); everything it decides is in the pure
//! modules next to it.

pub mod entry;
pub mod hook;
pub mod il2cpp;
pub mod pipe;
pub mod pump;
pub mod threads;

use crate::log::{Level, LogSink, RotatingLog, LOG_CAP_BYTES};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

/// A NUL-terminated UTF-16 string for the Win32 `W` functions.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The wall clock the worker waits with.
pub struct RealClock;

impl crate::slot::Clock for RealClock {
    fn now_ms(&self) -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

static LOG: Mutex<Option<RotatingLog>> = Mutex::new(None);

/// The process-wide log (the worker, the walk and the panic hook all write to it).
#[derive(Clone, Copy, Default)]
pub struct GlobalLog;

impl GlobalLog {
    /// Opens `ingame.log` in `dir`.
    pub fn install(dir: &Path) {
        *LOG.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(RotatingLog::open(dir, LOG_CAP_BYTES));
    }

    /// Closes the log; later writes do nothing.
    pub fn close() {
        *LOG.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

impl LogSink for GlobalLog {
    fn log(&mut self, level: Level, message: &str) {
        if let Some(log) = LOG.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            log.log(level, message);
        }
    }
}
