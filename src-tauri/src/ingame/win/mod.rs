//! The Win32 side of in-game sending (Windows only): process and TCP lookups, the
//! remote load, and the pipe. Only compiled, never unit-tested here.

pub mod host;
pub mod loader;
pub mod pipe;
pub mod process;
pub mod tcp;

use resonance_core::ingame_send::{
    connect, exchange, list_games, map_send_reply, Listener, Resident, SendError, UNLOAD_WAIT,
};
use resonance_types::ingame_pipe::{Request, APP_WAIT_MS};
use resonance_types::SendChatChannel;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

/// A kernel handle, closed on drop.
pub struct Handle(HANDLE);

// SAFETY: a kernel handle names a kernel object and may be used and closed from any
// thread; the session that holds these is behind a mutex.
unsafe impl Send for Handle {}

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

/// The pids of the running game clients ([`list_games`] over the live process
/// list and the chat-port connections).
pub fn find_clients() -> Result<Vec<u32>, SendError> {
    let processes = process::list_processes();
    Ok(list_games(
        &processes,
        tcp::chat_port_owners,
        process::is_32bit,
    )?)
}

/// A connected session: the pipe the DLL answers on, and the game it lives in.
pub struct Session {
    pid: u32,
    pipe: pipe::Pipe,
    next_id: u32,
}

impl Session {
    /// Loads the DLL file `dll` into the game `pid` and says `hello` ([`connect`]).
    pub fn connect(
        pid: u32,
        resident: &mut Resident,
        dll: &std::path::Path,
    ) -> Result<Session, SendError> {
        let mut host = host::WinHost::new(dll.to_path_buf());
        let pid = connect(&mut host, resident, pid)?;
        let pipe = host.pipe.take().ok_or(SendError::PipeCreate)?;
        Ok(Session {
            pid,
            pipe,
            next_id: host.next_id,
        })
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Sends one chat line. The flag says the session is no longer usable.
    pub fn send(&mut self, channel: SendChatChannel, text: &str) -> (Result<(), SendError>, bool) {
        let request = Request::Send {
            id: self.id(),
            channel: channel.as_str().to_string(),
            text: text.to_string(),
        };
        let wait = Duration::from_millis(u64::from(APP_WAIT_MS));
        match exchange(&mut self.pipe, &request, wait) {
            Ok(reply) => map_send_reply(reply),
            Err(e) => (Err(e), true),
        }
    }

    /// `bye`, then waits for the module to leave the game; the pipe closes whatever
    /// happened. A module that stays is logged and its game pid returned, so the
    /// caller can remember it ([`Resident`]); it is not an error for the user.
    pub fn unload(mut self) -> Option<u32> {
        let id = self.id();
        let wait = Duration::from_millis(u64::from(APP_WAIT_MS));
        if exchange(&mut self.pipe, &Request::Bye { id }, wait).is_err() {
            // No answer: the DLL unloads itself when it sees the pipe close, so
            // close it before waiting rather than after.
            self.pipe.disconnect();
        }
        let stayed = !loader::wait_module_gone(self.pid, UNLOAD_WAIT);
        if stayed {
            log::warn!("the in-game module is still loaded after bye");
        }
        self.pipe.disconnect();
        stayed.then_some(self.pid)
    }
}
