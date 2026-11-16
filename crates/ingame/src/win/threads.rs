//! Suspending the game's other threads around a code patch, as ilhook's `ThreadCallback`
//! (install) and directly (removal).
//!
//! The window between the first `SuspendThread` and the last `ResumeThread` runs only kernel
//! calls that take no user-mode lock (`SuspendThread`, `GetThreadContext`, `ResumeThread`),
//! integer comparisons, atomic loads and the 14-byte write: no allocation, no lock, no log,
//! no `std::thread` call. A suspended game thread may hold the heap or loader lock, so
//! anything that allocates happens before `stop` and after `resume`.
//!
//! NOT VERIFIED until Kade's run: that suspending the game's threads for these few
//! milliseconds causes no visible hitch, deadlock or anti-cheat reaction.

use crate::patch_safety::{
    patch_blocked, pick_main_thread, threads_of, unhook_blocked, Refusal, RefusalCell,
    TooManyThreads, MAX_SUSPEND_THREADS,
};
use ilhook::x64::ThreadCallback;
use std::mem::{size_of, zeroed};
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    GetThreadContext, CONTEXT, CONTEXT_CONTROL_AMD64,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Threading::{
    GetThreadTimes, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT,
    THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
};

/// `GetThreadContext` on x86_64 fails (`ERROR_NOACCESS`) unless the `CONTEXT` is 16-byte
/// aligned, and windows-sys declares it without `align(16)`.
#[repr(C, align(16))]
struct ContextBuf(CONTEXT);

/// The suspended thread's instruction pointer, or `GetLastError()` when the read failed.
/// No allocation: it runs while the game's threads are stopped.
fn read_rip(handle: HANDLE) -> Result<usize, u32> {
    // SAFETY: an all-integer struct, 16-aligned by `ContextBuf`.
    let mut buf: ContextBuf = unsafe { zeroed() };
    buf.0.ContextFlags = CONTEXT_CONTROL_AMD64;
    // SAFETY: `handle` has THREAD_GET_CONTEXT and its thread is suspended.
    if unsafe { GetThreadContext(handle, &mut buf.0) } != 0 {
        Ok(buf.0.Rip as usize)
    } else {
        // SAFETY: plain call, right after the failure.
        Err(unsafe { GetLastError() })
    }
}

pub struct SuspendState {
    handles: [AtomicUsize; MAX_SUSPEND_THREADS],
    tids: [AtomicU32; MAX_SUSPEND_THREADS],
    suspended: [AtomicBool; MAX_SUSPEND_THREADS],
    count: AtomicUsize,
    /// The bytes being rewritten.
    window: Range<usize>,
    /// Where the patch jumps to (set before a removal).
    tramp: AtomicUsize,
    dll: Range<usize>,
    in_detour: &'static AtomicUsize,
    /// Set once the code is restored: ilhook's own, later unhook then suspends nothing.
    unhooked: AtomicBool,
    /// Why the last `stop` refused; the caller logs it after `resume`.
    refused: RefusalCell,
}

impl SuspendState {
    pub fn new(window: Range<usize>, dll: Range<usize>, in_detour: &'static AtomicUsize) -> Self {
        Self {
            handles: [const { AtomicUsize::new(0) }; MAX_SUSPEND_THREADS],
            tids: [const { AtomicU32::new(0) }; MAX_SUSPEND_THREADS],
            suspended: [const { AtomicBool::new(false) }; MAX_SUSPEND_THREADS],
            count: AtomicUsize::new(0),
            window,
            tramp: AtomicUsize::new(0),
            dll,
            in_detour,
            unhooked: AtomicBool::new(false),
            refused: RefusalCell::new(),
        }
    }

    pub fn set_tramp(&self, tramp: usize) {
        self.tramp.store(tramp, Ordering::Release);
    }

    pub fn mark_unhooked(&self) {
        self.unhooked.store(true, Ordering::Release);
    }

    /// Why the last `stop` refused, once; clears it.
    pub fn take_refused(&self) -> Option<Refusal> {
        self.refused.take()
    }

    /// Opens a handle to every other thread of this process (the worker's own is left out);
    /// the result is how many could not be opened (and so are neither suspended nor
    /// checked). Allocates: never call it between `stop` and `resume`.
    pub fn prepare(&self, pid: u32, own_tid: u32) -> Result<usize, TooManyThreads> {
        self.close_handles();
        let mut entries: Vec<(u32, u32)> = Vec::new();
        // SAFETY: a thread snapshot; the handle is closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot != INVALID_HANDLE_VALUE {
            // SAFETY: an all-integer struct; `dwSize` is set as the API requires.
            let mut entry: THREADENTRY32 = unsafe { zeroed() };
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            // SAFETY: `snapshot` is valid and `entry` is a correctly sized THREADENTRY32.
            let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
            while more {
                entries.push((entry.th32OwnerProcessID, entry.th32ThreadID));
                // SAFETY: as above.
                more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
            }
            // SAFETY: `snapshot` came from `CreateToolhelp32Snapshot`.
            unsafe { CloseHandle(snapshot) };
        }
        let tids = threads_of(&entries, pid, own_tid, MAX_SUSPEND_THREADS)?;
        let mut n = 0;
        let mut skipped = 0;
        for tid in tids {
            // SAFETY: plain call; a null result (thread gone, access denied) is skipped.
            let handle = unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_LIMITED_INFORMATION,
                    0,
                    tid,
                )
            };
            if handle.is_null() {
                skipped += 1;
                continue;
            }
            self.handles[n].store(handle as usize, Ordering::Release);
            self.tids[n].store(tid, Ordering::Release);
            n += 1;
        }
        self.count.store(n, Ordering::Release);
        Ok(skipped)
    }

    /// The prepared thread created first: the process's first thread, taken to be the
    /// game's main thread (estimated, NOT VERIFIED).
    pub fn main_thread(&self) -> Option<u32> {
        let mut times = Vec::new();
        for i in 0..self.count.load(Ordering::Acquire) {
            let handle = self.handles[i].load(Ordering::Acquire) as HANDLE;
            let zero = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let (mut created, mut a, mut b, mut c) = (zero, zero, zero, zero);
            // SAFETY: `handle` was opened with THREAD_QUERY_LIMITED_INFORMATION.
            if unsafe { GetThreadTimes(handle, &mut created, &mut a, &mut b, &mut c) } != 0 {
                let time =
                    (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
                times.push((self.tids[i].load(Ordering::Acquire), time));
            }
        }
        pick_main_thread(&times)
    }

    /// Suspends every prepared thread and checks where each is. Returns `true` with the
    /// threads still suspended when it is safe to write; otherwise resumes them all and
    /// returns `false`. Nothing here allocates, locks or logs.
    pub fn stop(&self, unhooking: bool) -> bool {
        let count = self.count.load(Ordering::Acquire);
        let mut rips = [0usize; MAX_SUSPEND_THREADS];
        let mut known = 0;
        let mut unknown = false;
        for i in 0..count {
            let handle = self.handles[i].load(Ordering::Acquire) as HANDLE;
            // SAFETY: `handle` has THREAD_SUSPEND_RESUME; u32::MAX means it failed.
            if unsafe { SuspendThread(handle) } == u32::MAX {
                continue; // the thread is gone
            }
            self.suspended[i].store(true, Ordering::Release);
            match read_rip(handle) {
                Ok(rip) => {
                    rips[known] = rip;
                    known += 1;
                }
                Err(code) => {
                    unknown = true;
                    self.refused
                        .set_context(self.tids[i].load(Ordering::Acquire), code);
                }
            }
        }
        let ips = &rips[..known];
        let blocked = unknown
            || if unhooking {
                unhook_blocked(
                    ips,
                    self.window.clone(),
                    self.tramp.load(Ordering::Acquire),
                    self.dll.clone(),
                    self.in_detour.load(Ordering::Acquire),
                )
            } else {
                patch_blocked(ips, self.window.clone())
            };
        if blocked {
            self.resume();
            if !unknown {
                self.refused.set_inside();
            }
        }
        !blocked
    }

    /// Resumes every thread `stop` suspended.
    pub fn resume(&self) {
        for i in 0..self.count.load(Ordering::Acquire) {
            if self.suspended[i].swap(false, Ordering::AcqRel) {
                let handle = self.handles[i].load(Ordering::Acquire) as HANDLE;
                // SAFETY: `handle` is open and its thread was suspended by `stop`.
                unsafe { ResumeThread(handle) };
            }
        }
    }

    fn close_handles(&self) {
        self.resume();
        for i in 0..self.count.swap(0, Ordering::AcqRel) {
            let handle = self.handles[i].swap(0, Ordering::AcqRel) as HANDLE;
            // SAFETY: opened by `prepare`, closed once.
            unsafe { CloseHandle(handle) };
        }
    }
}

impl Drop for SuspendState {
    /// No thread stays frozen and no handle leaks, even after a panic between `pre` and
    /// `post`.
    fn drop(&mut self) {
        self.close_handles();
    }
}

/// ilhook's thread callback over a shared [`SuspendState`].
pub struct Callback(pub Arc<SuspendState>);

impl ThreadCallback for Callback {
    fn pre(&self) -> bool {
        if self.0.unhooked.load(Ordering::Acquire) {
            return true;
        }
        catch_unwind(AssertUnwindSafe(|| self.0.stop(false))).unwrap_or_else(|_| {
            self.0.resume();
            false
        })
    }

    fn post(&self) {
        self.0.resume();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use windows_sys::Win32::System::Threading::GetThreadId;

    #[test]
    fn context_buf_is_16_byte_aligned() {
        assert_eq!(std::mem::align_of::<ContextBuf>(), 16);
    }

    #[test]
    fn reads_the_rip_of_a_suspended_thread() {
        use std::os::windows::io::AsRawHandle;
        let (park_tx, park_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let join = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let _ = park_rx.recv();
        });
        ready_rx.recv().unwrap();
        // SAFETY: plain calls on the helper thread's own handle.
        let tid = unsafe { GetThreadId(join.as_raw_handle() as HANDLE) };
        let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT, 0, tid) };
        assert!(!handle.is_null());
        assert_ne!(unsafe { SuspendThread(handle) }, u32::MAX);
        let rip = read_rip(handle);
        unsafe { ResumeThread(handle) };
        unsafe { CloseHandle(handle) };
        park_tx.send(()).unwrap();
        join.join().unwrap();
        assert!(matches!(rip, Ok(r) if r != 0), "{rip:?}");
    }
}
