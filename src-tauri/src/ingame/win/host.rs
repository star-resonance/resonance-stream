//! The Windows [`Host`]: ties the process lookup, the pipe and the remote load
//! into the sequence `resonance_core::ingame_send::connect` runs.

use super::{loader, pipe::Pipe, process, tcp};
use crate::ingame::pin;
use resonance_core::ingame_send::{
    accept_game, check_pin, exchange, pick_game, sha256_hex, Host, PinError, SendError,
    CONNECT_WAIT, DLL_NAME, UNLOAD_WAIT,
};
use resonance_types::ingame_pipe::{Reply, Request, APP_WAIT_MS, PIPE_PROTOCOL_VERSION};
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::Duration;

/// `FILE_SHARE_READ`: others may read the DLL file but not write or delete it.
const SHARE_READ_ONLY: u32 = 1;

/// A connection in the making; after a successful `connect` its pipe and id
/// counter move into a `Session`.
pub struct WinHost {
    pub(super) pid: Option<u32>,
    pub(super) pipe: Option<Pipe>,
    /// The DLL file, held open from the hash until the load has finished.
    file: Option<File>,
    pub(super) next_id: u32,
}

impl WinHost {
    pub fn new() -> Self {
        WinHost {
            pid: None,
            pipe: None,
            file: None,
            next_id: 1,
        }
    }

    fn dll_path() -> Option<PathBuf> {
        if let Some(path) = pin::dll_path_override() {
            return Some(PathBuf::from(path));
        }
        let exe = std::env::current_exe().ok()?;
        Some(exe.parent()?.join("ingame").join(DLL_NAME))
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn pipe(&mut self) -> Result<&mut Pipe, SendError> {
        self.pipe.as_mut().ok_or(SendError::PipeCreate)
    }
}

impl Default for WinHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for WinHost {
    fn find_game(&mut self) -> Result<u32, SendError> {
        let processes = process::list_processes();
        let pid = pick_game(&processes, tcp::chat_port_owners, process::is_32bit)?;
        self.pid = Some(pid);
        Ok(pid)
    }

    fn create_pipe(&mut self, pid: u32) -> Result<(), SendError> {
        // A pipe of an earlier try is closed first so its name is free.
        self.pipe = None;
        self.pipe = Some(Pipe::create(pid)?);
        Ok(())
    }

    fn module_loaded(&mut self, pid: u32) -> bool {
        loader::module_loaded(pid)
    }

    fn wait_module_gone(&mut self, pid: u32, wait: Duration) -> bool {
        loader::wait_module_gone(pid, wait)
    }

    fn verify_dll(&mut self) -> Result<(), SendError> {
        if check_pin(pin::INGAME_DLL_SHA256, "") == Err(PinError::NoPin) {
            return Err(SendError::NoPin);
        }
        let path = Self::dll_path().ok_or(SendError::DllMissing)?;
        self.file = None;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_ONLY)
            .open(&path)
            .map_err(|_| SendError::DllMissing)?;
        let hash = sha256_hex(&file).map_err(|_| SendError::DllMissing)?;
        check_pin(pin::INGAME_DLL_SHA256, &hash)?;
        self.file = Some(file);
        Ok(())
    }

    fn load_dll(&mut self, pid: u32) -> Result<(), SendError> {
        let path = Self::dll_path().ok_or(SendError::DllMissing)?;
        // The verified file is still held open here: nothing can replace it
        // between the hash and the load. It is released once the load is over.
        let result = loader::load_remote(pid, &path);
        self.file = None;
        result
    }

    fn accept(&mut self, pid: u32) -> Result<(), SendError> {
        accept_game(self.pipe()?, pid, CONNECT_WAIT)
    }

    fn hello(&mut self) -> Result<Reply, SendError> {
        let id = self.id();
        let request = Request::Hello {
            id,
            version: PIPE_PROTOCOL_VERSION,
        };
        exchange(
            self.pipe()?,
            &request,
            Duration::from_millis(u64::from(APP_WAIT_MS)),
        )
    }

    fn bye_and_wait(&mut self, pid: u32) {
        let id = self.id();
        if let Some(pipe) = self.pipe.as_mut() {
            let _ = exchange(
                pipe,
                &Request::Bye { id },
                Duration::from_millis(u64::from(APP_WAIT_MS)),
            );
            // The DLL closed its end; the next client needs a fresh connect.
            resonance_core::ingame_send::Listener::disconnect(pipe);
        }
        if !loader::wait_module_gone(pid, UNLOAD_WAIT) {
            log::warn!("the in-game module is still loaded after bye");
        }
    }
}
