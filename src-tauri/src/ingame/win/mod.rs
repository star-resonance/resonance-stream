//! The Win32 side of in-game sending (Windows only): process and TCP lookups, the
//! remote load, and the pipe. Only compiled, never unit-tested here.

pub mod host;
pub mod loader;
pub mod pipe;
pub mod process;
pub mod tcp;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

/// A kernel handle, closed on drop.
pub struct Handle(HANDLE);

impl Handle {
    /// Wraps `raw`; `None` for a null or invalid handle.
    pub fn new(raw: HANDLE) -> Option<Handle> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Handle(raw))
        }
    }

    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and closed once.
        unsafe { CloseHandle(self.0) };
    }
}
