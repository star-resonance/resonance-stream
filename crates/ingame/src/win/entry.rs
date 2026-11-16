//! `DllMain`, the worker thread and the unload. `DllMain` only starts the worker (the loader
//! lock is held there, so no il2cpp call and no waiting); the worker resolves the game's API,
//! attaches, connects the pipe and serves it. A `bye`, a closed pipe and a failed connect
//! all end in the same unload.

use super::il2cpp::GameProbe;
use super::pipe::{PipeClient, ReadError};
use super::GlobalLog;
use crate::log::LogSink;
use crate::session::{Next, Session};
use crate::unload::{Unload, UnloadSteps};
use std::ffi::c_void;
use std::os::windows::ffi::OsStringExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, FreeLibraryAndExitThread, GetModuleFileNameW,
};
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_sys::Win32::System::Threading::{CreateThread, GetCurrentProcessId};

/// This DLL's module handle, kept from `DllMain` for `FreeLibraryAndExitThread`.
static MODULE: AtomicUsize = AtomicUsize::new(0);

/// # Safety
/// Called by the loader only.
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: HMODULE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    // Never unwind into the loader.
    let _ = catch_unwind(|| {
        if reason == DLL_PROCESS_ATTACH {
            MODULE.store(module as usize, Ordering::SeqCst);
            // SAFETY: `module` is this DLL; the worker is started and its handle closed.
            unsafe {
                DisableThreadLibraryCalls(module);
                let thread = CreateThread(null(), 0, Some(worker), null(), 0, null_mut());
                if !thread.is_null() {
                    CloseHandle(thread);
                }
            }
        }
    });
    1
}

/// What the worker holds, and what the unload closes.
#[derive(Default)]
struct Worker {
    pipe: Option<PipeClient>,
    probe: Option<GameProbe>,
}

impl UnloadSteps for Worker {
    fn stop_accepting(&mut self) {
        GlobalLog.info("unload: stop accepting");
    }
    fn disable_hooks(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn drain(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn close_pipe(&mut self) {
        self.pipe = None;
    }
    fn detach_thread(&mut self) {
        if let Some(probe) = self.probe.as_mut() {
            probe.detach();
        }
    }
    fn close_log(&mut self) {
        GlobalLog.info("unload: done");
        GlobalLog::close();
    }
}

unsafe extern "system" fn worker(_: *mut c_void) -> u32 {
    let mut state = Worker::default();
    let _ = catch_unwind(AssertUnwindSafe(|| run(&mut state)));
    // The same sequence however the run ended.
    Unload::new().run(&mut state);
    let module = MODULE.load(Ordering::SeqCst) as HMODULE;
    drop(state);
    // SAFETY: frees this thread's reference to the DLL and ends the thread; nothing of the
    // DLL runs afterwards.
    unsafe { FreeLibraryAndExitThread(module, 0) }
}

/// The folder this DLL was loaded from: `ingame.log` goes there.
fn own_folder() -> Option<PathBuf> {
    let module = MODULE.load(Ordering::SeqCst) as HMODULE;
    let mut buf = [0u16; 1024];
    // SAFETY: `buf` is valid for `buf.len()` units.
    let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n]));
    path.parent().map(PathBuf::from)
}

fn run(state: &mut Worker) {
    let dir = own_folder();
    if let Some(dir) = &dir {
        GlobalLog::install(dir);
    }
    // This DLL's own copy of std: a panic goes to the log, not to stderr.
    std::panic::set_hook(Box::new(|info| {
        GlobalLog.warn(&format!("panic: {info}"));
    }));
    let mut log = GlobalLog;
    log.info(&format!(
        "resonance_ingame {} loaded from {}",
        env!("CARGO_PKG_VERSION"),
        dir.as_ref().map_or("?".into(), |d| d.display().to_string())
    ));

    state.probe = Some(GameProbe::open(&mut log));
    // SAFETY: no arguments.
    let pid = unsafe { GetCurrentProcessId() };
    state.pipe = PipeClient::connect(pid, &mut log);
    serve(state, &mut log);
}

fn serve(state: &mut Worker, log: &mut GlobalLog) {
    let (Some(pipe), Some(probe)) = (state.pipe.as_mut(), state.probe.as_mut()) else {
        return;
    };
    let mut session = Session::new();
    loop {
        let lines = match pipe.read_lines() {
            Ok(lines) => lines,
            Err(ReadError::TooLong) => {
                let outcome = session.line_too_long(log);
                pipe.write(&outcome.reply);
                return;
            }
            Err(ReadError::Broken) => {
                log.info("pipe closed");
                return;
            }
        };
        for line in lines {
            let outcome = session.handle_line(&line, probe, log);
            if !pipe.write(&outcome.reply) {
                log.warn("pipe write failed; unloading");
                return;
            }
            if outcome.next != Next::Continue {
                return;
            }
        }
    }
}
