//! The ilhook detour on the pump's method: install, and a removal the DLL does itself.
//!
//! Why not `HookPoint::unhook`: it unpatches, then drops the `HookPoint`, whose `Drop`
//! unpatches again and frees the trampoline whether or not either unpatch was allowed to
//! run. A refused unpatch would leave the game jumping into freed memory. So the DLL
//! restores the 14 original bytes it read before hooking, under the same suspension rules,
//! and drops the `HookPoint` only afterwards; if the restore cannot be done, the hook (and
//! its trampoline) stays alive and the DLL stays resident.

use super::pump::{pump_detour, IN_DETOUR, MAIN_TID};
use super::threads::{Callback, SuspendState};
use crate::log::LogSink;
use crate::patch_safety::{
    decode_jmp_target, install_with_retry, InstallError, INSTALL_ATTEMPTS, RETRY_SLEEP_MS,
};
use crate::walk::PumpTarget;
use ilhook::x64::{CallbackOption, HookFlags, HookPoint, HookType, Hooker};
use ilhook::HookError;
use std::mem::ManuallyDrop;
use std::ops::Range;
use std::ptr::copy_nonoverlapping;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, Sleep,
};

/// Bytes ilhook rewrites at the hooked address.
const WINDOW_LEN: usize = 14;

pub struct PumpHook {
    hook: ManuallyDrop<HookPoint>,
    state: Arc<SuspendState>,
    addr: usize,
    original: [u8; WINDOW_LEN],
    pub name: String,
    pub main_tid: u32,
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl PumpHook {
    /// Hooks the first candidate ilhook accepts. `dll` is this DLL's image (a thread inside
    /// it blocks a removal). Everything that allocates happens outside the suspension.
    pub fn install(
        targets: &[PumpTarget],
        dll: Range<usize>,
        log: &mut dyn LogSink,
    ) -> Result<PumpHook, InstallError> {
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        let log_cell = std::cell::RefCell::new(log);
        let result = install_with_retry(
            &names,
            |name| {
                let target = targets
                    .iter()
                    .find(|t| t.name == name)
                    .ok_or_else(|| InstallError::Unsupported("unknown candidate".into()))?;
                Self::try_install(target, dll.clone(), &mut **log_cell.borrow_mut())
            },
            |ms| {
                // SAFETY: plain sleep, outside any suspension.
                unsafe { Sleep(ms as u32) };
            },
        );
        let log = log_cell.into_inner();
        match result {
            Ok((name, hook)) => {
                log.info(&format!(
                    "hook installed on {name} at {:#x}; original bytes {}; main thread {}",
                    hook.addr,
                    hex(&hook.original),
                    hook.main_tid
                ));
                Ok(hook)
            }
            Err(e) => {
                log.warn(&format!("hook install failed: {e:?}"));
                Err(e)
            }
        }
    }

    fn try_install(
        target: &PumpTarget,
        dll: Range<usize>,
        log: &mut dyn LogSink,
    ) -> Result<PumpHook, InstallError> {
        let addr = target.address;
        // SAFETY: the address is the method's native code pointer; reading its first bytes
        // is what ilhook does too.
        let original: [u8; WINDOW_LEN] = unsafe { std::ptr::read_unaligned(addr as *const _) };
        let state = Arc::new(SuspendState::new(addr..addr + WINDOW_LEN, dll, &IN_DETOUR));
        // SAFETY: plain calls.
        let (pid, tid) = unsafe { (GetCurrentProcessId(), GetCurrentThreadId()) };
        let skipped = state
            .prepare(pid, tid)
            .map_err(|_| InstallError::Unsupported("too_many_threads".into()))?;
        if skipped > 0 {
            log.warn(&format!("hook: {skipped} thread(s) could not be opened"));
        }
        // No thread to pick from may pass; try again rather than caching a failure.
        let main_tid = state.main_thread().ok_or(InstallError::Transient)?;
        MAIN_TID.store(main_tid, Ordering::Release);
        let hooker = Hooker::new(
            addr,
            HookType::JmpBack(pump_detour),
            CallbackOption::Some(Box::new(Callback(state.clone()))),
            0,
            HookFlags::empty(),
        );
        // SAFETY: `addr` is the start of a game method whose prologue ilhook disassembles
        // and refuses if it cannot relocate; the worker is the only thread that installs or
        // removes hooks, and the other threads are suspended around the 14-byte write.
        match unsafe { hooker.hook() } {
            Ok(hook) => Ok(PumpHook {
                hook: ManuallyDrop::new(hook),
                state,
                addr,
                original,
                name: target.name.clone(),
                main_tid,
            }),
            Err(HookError::PreHook) => {
                if state.take_refused() {
                    log.info("hook: a thread was inside the 14 bytes; retrying");
                }
                Err(InstallError::Transient)
            }
            Err(e) => Err(InstallError::Unsupported(format!("{e:?}"))),
        }
    }

    /// Restores the original bytes with the other threads suspended and none inside the
    /// patch, the trampoline, the DLL or a detour; then frees the trampoline. `Err` leaves
    /// the hook in place (the DLL must stay loaded).
    pub fn remove(mut self, log: &mut dyn LogSink) -> Result<(), PumpHook> {
        // Where the patch jumps to: the trampoline.
        let mut window = [0u8; WINDOW_LEN];
        // SAFETY: the 14 bytes at the hooked address are readable (we patched them).
        unsafe { copy_nonoverlapping(self.addr as *const u8, window.as_mut_ptr(), WINDOW_LEN) };
        // Without the trampoline's address a thread inside it cannot be found: do not remove.
        let Some(tramp) = decode_jmp_target(&window, self.addr) else {
            log.warn("unhook: cannot tell where the patch jumps to; keeping the hook");
            return Err(self);
        };
        self.state.set_tramp(tramp);
        let (mut old, mut scratch) = (0u32, 0u32);
        // SAFETY: the page of the hooked code; protection is restored below.
        let writable = unsafe {
            VirtualProtect(
                self.addr as *const _,
                WINDOW_LEN,
                PAGE_EXECUTE_READWRITE,
                &mut old,
            )
        } != 0;
        if !writable {
            log.warn("unhook: VirtualProtect failed");
            return Err(self);
        }
        let mut restored = false;
        for attempt in 1..=INSTALL_ATTEMPTS {
            // SAFETY: plain calls.
            let (pid, tid) = unsafe { (GetCurrentProcessId(), GetCurrentThreadId()) };
            if self.state.prepare(pid, tid).is_err() {
                log.warn("unhook: too many threads");
                break;
            }
            if self.state.stop(true) {
                // SAFETY: the 14 bytes are writable (above); every other thread is
                // suspended and none is inside them.
                unsafe {
                    copy_nonoverlapping(self.original.as_ptr(), self.addr as *mut u8, WINDOW_LEN)
                };
                self.state.resume();
                restored = true;
                break;
            }
            if self.state.take_refused() {
                log.info(&format!("unhook: a thread was in the way (try {attempt})"));
            }
            // SAFETY: plain sleep, outside any suspension.
            unsafe { Sleep(RETRY_SLEEP_MS as u32) };
        }
        // SAFETY: restoring the page protection and flushing; plain calls.
        unsafe {
            VirtualProtect(self.addr as *const _, WINDOW_LEN, old, &mut scratch);
            FlushInstructionCache(GetCurrentProcess(), self.addr as *const _, WINDOW_LEN);
        }
        if !restored {
            return Err(self);
        }
        log.info(&format!("unhook: original bytes restored on {}", self.name));
        self.state.mark_unhooked();
        // SAFETY: the code is restored and no thread was in the trampoline or the DLL when
        // it was; ilhook's `Drop` rewrites the same bytes and frees the trampoline.
        unsafe { ManuallyDrop::drop(&mut self.hook) };
        Ok(())
    }
}
