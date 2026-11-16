//! `DllMain`, the worker thread and the unload. `DllMain` only starts the worker (the loader
//! lock is held there, so no il2cpp call and no waiting); the worker resolves the game's API,
//! attaches, connects the pipe and serves it. A `bye`, a closed pipe and a failed connect
//! all end in the same unload.

use super::hook::PumpHook;
use super::il2cpp::{GameProbe, Il2cpp};
use super::pipe::{PipeClient, ReadError};
use super::pump::{set_ops, SlotQueue, IN_DETOUR, SLOT};
use super::{GlobalLog, RealClock};
use crate::log::LogSink;
use crate::patch_safety::{drain, InstallError};
use crate::send::{CallIds, SendOutcome, SendQueue, SendService};
use crate::session::{Next, Session};
use crate::unload::{Unload, UnloadOutcome, UnloadSteps};
use crate::walk::Walker;
use std::cell::RefCell;
use std::ffi::c_void;
use std::os::windows::ffi::OsStringExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, HMODULE};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows_sys::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, FreeLibraryAndExitThread, GetModuleFileNameW, GetModuleHandleW,
};
use windows_sys::Win32::System::ProcessStatus::{GetModuleInformation, MODULEINFO};
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_sys::Win32::System::Threading::{CreateThread, GetCurrentProcess, GetCurrentProcessId};

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

/// The pump hook's life: installed on the first send that passes the re-check, never at
/// `hello` (the game is not patched until a send is asked for).
enum HookState {
    NotTried,
    /// Install failed for a reason that will not change; every send answers it.
    Failed(SendOutcome),
    Installed(PumpHook),
    /// The hook could not be removed: it stays, and so does the DLL.
    Stuck(PumpHook),
    Removed,
}

type Hook = Rc<RefCell<HookState>>;

/// This DLL's image, for the removal's "no thread inside the DLL" check.
fn dll_range() -> std::ops::Range<usize> {
    let module = MODULE.load(Ordering::SeqCst) as HMODULE;
    // SAFETY: an all-integer struct.
    let mut info: MODULEINFO = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a MODULEINFO and the size passed is its own.
    let ok = unsafe {
        GetModuleInformation(
            GetCurrentProcess(),
            module,
            &mut info,
            std::mem::size_of::<MODULEINFO>() as u32,
        )
    };
    if ok == 0 {
        return 0..0;
    }
    let base = info.lpBaseOfDll as usize;
    base..base + info.SizeOfImage as usize
}

/// The queue the send service uses: installs the hook on first use, then hands the frame to
/// the pump through the slot.
struct HookingQueue {
    hook: Hook,
    /// The game's il2cpp binding for the pump (not attached by this DLL), if it resolved.
    il2cpp: Option<Il2cpp>,
}

impl HookingQueue {
    fn ensure_hook(&mut self) -> Result<(), SendOutcome> {
        match &*self.hook.borrow() {
            HookState::Installed(_) => return Ok(()),
            HookState::Failed(outcome) => return Err(outcome.clone()),
            HookState::Stuck(_) | HookState::Removed => return Err(SendOutcome::Disabled),
            HookState::NotTried => {}
        }
        let mut log = GlobalLog;
        let fail = |outcome: SendOutcome, cache: bool, hook: &Hook| {
            if cache {
                *hook.borrow_mut() = HookState::Failed(outcome.clone());
            }
            Err(outcome)
        };
        let Some(il2cpp) = self.il2cpp.as_ref() else {
            return fail(
                SendOutcome::Unsupported("resolution failed".into()),
                true,
                &self.hook,
            );
        };
        let targets = match Walker::new(il2cpp.unattached()).resolve_pumps(&mut log) {
            Ok(targets) => targets,
            Err(e) => return fail(SendOutcome::Unsupported(e.detail()), true, &self.hook),
        };
        set_ops(il2cpp.unattached());
        match PumpHook::install(&targets, dll_range(), &mut log) {
            Ok(hook) => {
                *self.hook.borrow_mut() = HookState::Installed(hook);
                Ok(())
            }
            // A thread was in the way every time: try again on the next send.
            Err(InstallError::Transient) => {
                fail(SendOutcome::HookFailed("PreHook".into()), false, &self.hook)
            }
            Err(InstallError::Unsupported(why)) => {
                fail(SendOutcome::HookFailed(why), true, &self.hook)
            }
        }
    }
}

impl SendQueue for HookingQueue {
    fn submit(&mut self, frame: &[u8]) -> SendOutcome {
        match self.ensure_hook() {
            Ok(()) => SlotQueue.submit(frame),
            Err(outcome) => outcome,
        }
    }
}

/// What the worker holds, and what the unload closes.
struct Worker {
    pipe: Option<PipeClient>,
    probe: Option<GameProbe>,
    hook: Hook,
}

impl Default for Worker {
    fn default() -> Self {
        Self {
            pipe: None,
            probe: None,
            hook: Rc::new(RefCell::new(HookState::NotTried)),
        }
    }
}

impl UnloadSteps for Worker {
    fn stop_accepting(&mut self) {
        GlobalLog.info("unload: stop accepting");
        SLOT.close();
    }
    fn disable_hooks(&mut self) -> Result<(), String> {
        let state = std::mem::replace(&mut *self.hook.borrow_mut(), HookState::Removed);
        match state {
            HookState::Installed(hook) | HookState::Stuck(hook) => {
                match hook.remove(&mut GlobalLog) {
                    Ok(()) => Ok(()),
                    Err(hook) => {
                        *self.hook.borrow_mut() = HookState::Stuck(hook);
                        Err("hook still patched".into())
                    }
                }
            }
            other => {
                *self.hook.borrow_mut() = other;
                Ok(())
            }
        }
    }
    fn drain(&mut self) -> Result<(), String> {
        drain(&|| IN_DETOUR.load(Ordering::SeqCst), &RealClock)
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
    let outcome = Unload::new().run(&mut state);
    let module = MODULE.load(Ordering::SeqCst) as HMODULE;
    drop(state);
    if outcome == UnloadOutcome::KeepResident {
        // A hook may still jump into this DLL: it stays loaded, only this thread ends.
        return 0;
    }
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

    log_build_key(&mut log);
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
    // Which hook candidates resolve: resolution only, nothing is patched.
    match probe.resolve_pumps(&mut *log) {
        Ok(targets) => {
            let names: Vec<_> = targets.iter().map(|t| t.name.as_str()).collect();
            log.info(&format!(
                "hook candidates that resolve: {}",
                names.join(", ")
            ));
        }
        Err(e) => log.info(&format!("hook candidates: {e}")),
    }
    let mut sender = SendService::new(
        HookingQueue {
            hook: state.hook.clone(),
            il2cpp: probe.pump_il2cpp(),
        },
        CallIds::new(),
    );
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
            let outcome = session.handle_line(&line, probe, &mut sender, log);
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

/// A module's full path as UTF-16 (NUL-terminated): the exe for a null module.
fn module_path(module: HMODULE) -> Option<Vec<u16>> {
    let mut buf = [0u16; 1024];
    // SAFETY: `buf` is valid for `buf.len()` units; null names the exe.
    let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    Some(buf[..=n].to_vec())
}

fn file_name(path: &[u16]) -> String {
    let path = PathBuf::from(std::ffi::OsString::from_wide(&path[..path.len() - 1]));
    path.file_name()
        .map_or("?".into(), |n| n.to_string_lossy().into_owned())
}

/// `a.b.c.d` from the file's version resource.
fn file_version(path: &[u16]) -> Option<String> {
    let mut handle = 0u32;
    // SAFETY: `path` is NUL-terminated.
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut handle) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: `buf` holds `size` bytes.
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, buf.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let root = super::wide("\\");
    let mut info: *mut c_void = null_mut();
    let mut len = 0u32;
    // SAFETY: `buf` is the version block read above; `root` is NUL-terminated.
    let ok = unsafe { VerQueryValueW(buf.as_ptr().cast(), root.as_ptr(), &mut info, &mut len) };
    if ok == 0 || info.is_null() || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    // SAFETY: `info` points into `buf` at a VS_FIXEDFILEINFO of at least that size.
    let v = unsafe { std::ptr::read_unaligned(info as *const VS_FIXEDFILEINFO) };
    Some(format!(
        "{}.{}.{}.{}",
        v.dwFileVersionMS >> 16,
        v.dwFileVersionMS & 0xFFFF,
        v.dwFileVersionLS >> 16,
        v.dwFileVersionLS & 0xFFFF
    ))
}

/// The game's build key, for matching a log to a game patch: the exe's file version and
/// `GameAssembly.dll`'s size and modification time. File names only, no folders.
fn log_build_key(log: &mut dyn LogSink) {
    let exe = module_path(null_mut());
    let exe_version = exe.as_deref().and_then(file_version);
    let exe_name = exe.as_deref().map_or("?".into(), file_name);
    let assembly_name = super::wide("GameAssembly.dll");
    // SAFETY: a NUL-terminated name.
    let module = unsafe { GetModuleHandleW(assembly_name.as_ptr()) };
    let assembly = if module.is_null() {
        None
    } else {
        module_path(module)
    };
    let (size, modified) = assembly
        .as_deref()
        .map(|p| PathBuf::from(std::ffi::OsString::from_wide(&p[..p.len() - 1])))
        .and_then(|p| std::fs::metadata(p).ok())
        .map_or((None, None), |m| {
            let secs = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            (Some(m.len()), secs)
        });
    log.info(&format!(
        "game build: {exe_name} version {} | GameAssembly.dll size {} modified {} (unix seconds)",
        exe_version.as_deref().unwrap_or("?"),
        size.map_or("?".into(), |n| n.to_string()),
        modified.map_or("?".into(), |n| n.to_string()),
    ));
}
