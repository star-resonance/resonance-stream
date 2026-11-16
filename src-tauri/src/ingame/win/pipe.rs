//! The named pipe the DLL connects to: a single instance created by the app before
//! the load, with a user-only DACL and a Medium label, driven with overlapped I/O.

use super::Handle;
use resonance_core::ingame_send::{pipe_sddl, Listener, SendError, Wire, WireError};
use resonance_types::ingame_pipe::pipe_name;
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, ResetEvent, WaitForSingleObject,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

const BUFFER_BYTES: u32 = 4096;

/// The current user's SID as a string (`S-1-5-21-...`).
fn user_sid_string() -> Option<String> {
    let mut token = std::ptr::null_mut();
    // SAFETY: opens the current process's token for query; `token` is valid for writes.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Handle::new(token)?;
    let mut needed: u32 = 0;
    // SAFETY: the size query: a null buffer of length 0 fails and sets `needed`.
    unsafe { GetTokenInformation(token.raw(), TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return None;
    }
    // 8-byte aligned storage for the TOKEN_USER and the SID that follows it.
    let mut storage: Vec<u64> = vec![0; (needed as usize).div_ceil(8)];
    // SAFETY: the buffer is at least `needed` bytes.
    let ok = unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: on success the buffer holds a TOKEN_USER whose SID lives in the same buffer.
    let sid = unsafe { (*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut wide: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is valid; the string is allocated by the call and freed below.
    if unsafe { ConvertSidToStringSidW(sid, &mut wide) } == 0 || wide.is_null() {
        return None;
    }
    // SAFETY: `wide` is a NUL-terminated string from ConvertSidToStringSidW.
    let text = unsafe {
        let mut len = 0;
        while *wide.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(wide, len))
    };
    // SAFETY: `wide` was allocated with LocalAlloc by the call above.
    unsafe { LocalFree(wide.cast()) };
    Some(text)
}

/// A security descriptor from `ConvertStringSecurityDescriptor...`, freed on drop.
struct Descriptor(*mut core::ffi::c_void);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated with LocalAlloc by the conversion call; freed once.
        unsafe { LocalFree(self.0) };
    }
}

/// How an overlapped operation ended.
enum IoEnd {
    Timeout,
    Broken,
}

pub struct Pipe {
    handle: Handle,
    event: Handle,
}

impl Pipe {
    /// Creates the first and only instance of the pipe for the game process `pid`.
    /// Fails (`PipeCreate`) when another process already holds the name.
    pub fn create(pid: u32) -> Result<Pipe, SendError> {
        let sid = user_sid_string().ok_or(SendError::PipeCreate)?;
        let sddl: Vec<u16> = pipe_sddl(&sid).encode_utf16().chain(Some(0)).collect();
        let mut raw: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; the descriptor is allocated by the call.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut raw,
                std::ptr::null_mut(),
            )
        };
        if converted == 0 || raw.is_null() {
            return Err(SendError::PipeCreate);
        }
        let descriptor = Descriptor(raw);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let name: Vec<u16> = pipe_name(pid).encode_utf16().chain(Some(0)).collect();
        // SAFETY: `name` is NUL-terminated and `attributes` outlives the call.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                BUFFER_BYTES,
                BUFFER_BYTES,
                0,
                &attributes,
            )
        };
        let handle = Handle::new(handle).ok_or(SendError::PipeCreate)?;
        // SAFETY: a manual-reset, initially non-signalled, unnamed event.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        let event = Handle::new(event).ok_or(SendError::PipeCreate)?;
        Ok(Pipe { handle, event })
    }

    fn overlapped(&self) -> OVERLAPPED {
        // SAFETY: OVERLAPPED is plain data; all-zero is its documented initial state.
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = self.event.raw();
        // SAFETY: a live event handle.
        unsafe { ResetEvent(self.event.raw()) };
        ov
    }

    /// Waits for the pending operation `ov`; on a timeout cancels it and waits for
    /// the cancel, so `ov` and any buffer are free when this returns. An operation
    /// that completed in that window counts as done.
    fn complete(&self, ov: &mut OVERLAPPED, wait: Duration) -> Result<u32, IoEnd> {
        let ms = wait.as_millis().min(u128::from(u32::MAX - 1)) as u32;
        // SAFETY: a live event handle.
        let waited = unsafe { WaitForSingleObject(self.event.raw(), ms) };
        let mut bytes: u32 = 0;
        if waited == WAIT_OBJECT_0 {
            // SAFETY: `ov` is the operation's own structure; it has completed.
            let ok = unsafe { GetOverlappedResult(self.handle.raw(), ov, &mut bytes, 0) };
            return if ok != 0 {
                Ok(bytes)
            } else {
                Err(IoEnd::Broken)
            };
        }
        // SAFETY: cancels exactly this operation, then waits for it to end.
        let ok = unsafe {
            CancelIoEx(self.handle.raw(), ov);
            GetOverlappedResult(self.handle.raw(), ov, &mut bytes, 1)
        };
        if ok != 0 {
            Ok(bytes)
        } else if waited == WAIT_TIMEOUT {
            Err(IoEnd::Timeout)
        } else {
            Err(IoEnd::Broken)
        }
    }
}

fn wire_error(end: IoEnd) -> WireError {
    match end {
        IoEnd::Timeout => WireError::Timeout,
        IoEnd::Broken => WireError::Broken,
    }
}

impl Listener for Pipe {
    fn wait_client(&mut self, wait: Duration) -> Result<Option<u32>, SendError> {
        let mut ov = self.overlapped();
        // SAFETY: `ov` lives until the operation has ended (see `complete`).
        let connected = unsafe { ConnectNamedPipe(self.handle.raw(), &mut ov) };
        if connected == 0 {
            // SAFETY: reads the calling thread's last error.
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => {}
                ERROR_IO_PENDING => match self.complete(&mut ov, wait) {
                    Ok(_) => {}
                    Err(IoEnd::Timeout) => return Ok(None),
                    Err(IoEnd::Broken) => return Err(SendError::Broken),
                },
                _ => return Err(SendError::Broken),
            }
        }
        let mut pid: u32 = 0;
        // SAFETY: the pipe is connected; `pid` is valid for writes.
        if unsafe { GetNamedPipeClientProcessId(self.handle.raw(), &mut pid) } == 0 {
            // Unknown client: report pid 0, which is never the game, so it is dropped.
            pid = 0;
        }
        Ok(Some(pid))
    }

    fn disconnect(&mut self) {
        // SAFETY: a live pipe handle.
        unsafe { DisconnectNamedPipe(self.handle.raw()) };
    }
}

impl Wire for Pipe {
    fn write_line(&mut self, line: &[u8], timeout: Duration) -> Result<(), WireError> {
        let mut ov = self.overlapped();
        // SAFETY: `line` and `ov` outlive the operation (see `complete`).
        let started = unsafe {
            WriteFile(
                self.handle.raw(),
                line.as_ptr(),
                line.len() as u32,
                std::ptr::null_mut(),
                &mut ov,
            )
        };
        // SAFETY: reads the calling thread's last error.
        if started == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
            return Err(WireError::Broken);
        }
        let written = self.complete(&mut ov, timeout).map_err(wire_error)?;
        if written as usize == line.len() {
            Ok(())
        } else {
            Err(WireError::Broken)
        }
    }

    fn read(&mut self, buf: &mut [u8], wait: Duration) -> Result<usize, WireError> {
        let mut ov = self.overlapped();
        // SAFETY: `buf` and `ov` outlive the operation (see `complete`).
        let started = unsafe {
            ReadFile(
                self.handle.raw(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                std::ptr::null_mut(),
                &mut ov,
            )
        };
        // SAFETY: reads the calling thread's last error.
        if started == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
            return Err(WireError::Broken);
        }
        let read = self.complete(&mut ov, wait).map_err(wire_error)?;
        Ok(read as usize)
    }
}
