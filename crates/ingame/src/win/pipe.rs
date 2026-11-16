//! The DLL's end of the app's named pipe. The app creates the pipe (with its DACL and its
//! client-pid check); the DLL only connects as the client and answers. Reads wait without a
//! timeout (the app speaks when it wants); each write waits [`WRITE_TIMEOUT_MS`], then is
//! cancelled and the pipe counts as broken.

use super::wide;
use crate::log::LogSink;
use resonance_types::ingame_pipe::{pipe_name, LineBuffer, WRITE_TIMEOUT_MS};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

/// How long to keep trying to connect (the app's load wait, a placeholder from PR #398).
const CONNECT_FOR: Duration = Duration::from_secs(5);
const CONNECT_EVERY: Duration = Duration::from_millis(250);

#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The pipe closed or failed.
    Broken,
    /// A line passed the cap.
    TooLong,
}

pub struct PipeClient {
    handle: HANDLE,
    event: HANDLE,
    lines: LineBuffer,
}

impl PipeClient {
    /// Connects to the app's pipe for this process, retrying while it is not there yet.
    pub fn connect(pid: u32, log: &mut dyn LogSink) -> Option<PipeClient> {
        let name = wide(&pipe_name(pid));
        let started = Instant::now();
        loop {
            // SAFETY: `name` is a NUL-terminated wide string; the other arguments are plain.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                // SAFETY: a manual-reset, unnamed event with default security.
                let event = unsafe { CreateEventW(null(), 1, 0, null()) };
                if event.is_null() {
                    log.warn("pipe: CreateEventW failed");
                    // SAFETY: `handle` was just opened and is not used again.
                    unsafe { CloseHandle(handle) };
                    return None;
                }
                let mut server = 0u32;
                // SAFETY: `handle` is an open pipe; `server` receives the pid.
                if unsafe { GetNamedPipeServerProcessId(handle, &mut server) } != 0 {
                    log.info(&format!("pipe connected; server pid {server}"));
                } else {
                    log.info("pipe connected; server pid unknown");
                }
                return Some(PipeClient {
                    handle,
                    event,
                    lines: LineBuffer::new(),
                });
            }
            // SAFETY: reads the calling thread's last error.
            let error = unsafe { GetLastError() };
            let retry = error == ERROR_FILE_NOT_FOUND || error == ERROR_PIPE_BUSY;
            if !retry || started.elapsed() >= CONNECT_FOR {
                log.warn(&format!("pipe: could not connect (error {error})"));
                return None;
            }
            std::thread::sleep(CONNECT_EVERY);
        }
    }

    /// Waits for bytes and returns the lines they completed (possibly none).
    pub fn read_lines(&mut self) -> Result<Vec<Vec<u8>>, ReadError> {
        let mut buf = [0u8; 1024];
        // SAFETY: an all-zero OVERLAPPED is valid; the event is ours.
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = self.event;
        // SAFETY: `buf` and `overlapped` outlive the call: a pending read is waited for below.
        let ok = unsafe {
            ReadFile(
                self.handle,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                null_mut(),
                &mut overlapped,
            )
        };
        if ok == 0 {
            // SAFETY: reads the last error of this thread.
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(ReadError::Broken);
            }
            // SAFETY: `event` is a valid event handle.
            if unsafe { WaitForSingleObject(self.event, INFINITE) } != WAIT_OBJECT_0 {
                return Err(ReadError::Broken);
            }
        }
        let mut n = 0u32;
        // SAFETY: the operation completed; `n` receives the byte count.
        if unsafe { GetOverlappedResult(self.handle, &overlapped, &mut n, 0) } == 0 || n == 0 {
            return Err(ReadError::Broken);
        }
        self.lines
            .push(&buf[..n as usize])
            .map_err(|_| ReadError::TooLong)
    }

    /// Writes all of `bytes` within the write deadline; `false` if the pipe is broken.
    pub fn write(&mut self, bytes: &[u8]) -> bool {
        // SAFETY: an all-zero OVERLAPPED is valid; the event is ours.
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = self.event;
        // SAFETY: `bytes` and `overlapped` outlive the call: a pending write is settled below.
        let ok = unsafe {
            WriteFile(
                self.handle,
                bytes.as_ptr().cast(),
                bytes.len() as u32,
                null_mut(),
                &mut overlapped,
            )
        };
        let mut wait = true;
        if ok == 0 {
            // SAFETY: reads the last error of this thread.
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return false;
            }
            // SAFETY: `event` is a valid event handle.
            if unsafe { WaitForSingleObject(self.event, WRITE_TIMEOUT_MS) } != WAIT_OBJECT_0 {
                // SAFETY: cancels this handle's pending request; the result is then awaited
                // so `overlapped` is not in use when it goes out of scope.
                unsafe { CancelIoEx(self.handle, &overlapped) };
                wait = false;
            }
        }
        let mut n = 0u32;
        // SAFETY: `n` receives the byte count; when cancelled this waits for the cancel.
        let done =
            unsafe { GetOverlappedResult(self.handle, &overlapped, &mut n, i32::from(!wait)) };
        done != 0 && n as usize == bytes.len()
    }
}

impl Drop for PipeClient {
    fn drop(&mut self) {
        // SAFETY: both handles are owned by this value and closed once.
        unsafe {
            CloseHandle(self.handle);
            CloseHandle(self.event);
        }
    }
}
