//! The pump's statics and the detour ilhook jumps to. The detour runs on whichever game
//! thread calls the hooked method (the main thread, if the guess holds): when idle it is one
//! atomic load; otherwise it counts itself in, runs one pump tick under `catch_unwind` (an
//! `extern "win64"` function must not unwind) and counts itself out.

use super::{GlobalLog, RealClock};
use crate::pump::{pump_once, PumpResult, SendOps};
use crate::send::{SendOutcome, SendQueue};
use crate::send_ops::Il2cppSendOps;
use crate::slot::SendSlot;
use crate::walk::Walker;
use ilhook::x64::Registers;
use std::panic::catch_unwind;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

pub static SLOT: SendSlot = SendSlot::new();
/// Detours running now, counted by the detour itself (a thread deep in game code called by
/// the pump is counted too).
pub static IN_DETOUR: AtomicUsize = AtomicUsize::new(0);
/// The thread the pump acts on.
pub static MAIN_TID: AtomicU32 = AtomicU32::new(0);

/// The ops the pump uses: its own walker over an il2cpp binding that is not attached by
/// this DLL (the game's main thread already is).
pub struct PumpOps(pub Il2cppSendOps<super::il2cpp::Il2cpp, GlobalLog>);

// SAFETY: the il2cpp pointers inside are only used from the game's main thread, one at a
// time (the `Mutex` below); they are not tied to the thread that created them.
unsafe impl Send for PumpOps {}

impl SendOps for PumpOps {
    fn live_client(&mut self) -> Result<usize, SendOutcome> {
        self.0.live_client()
    }
    fn send(&mut self, client: usize, frame: &[u8]) -> Result<(), SendOutcome> {
        self.0.send(client, frame)
    }
}

pub static OPS: OnceLock<Mutex<PumpOps>> = OnceLock::new();

/// Installs the ops the detour uses (once).
pub fn set_ops(il2cpp: super::il2cpp::Il2cpp) {
    let ops = PumpOps(Il2cppSendOps::new(Walker::new(il2cpp), GlobalLog));
    let _ = OPS.set(Mutex::new(ops));
}

/// # Safety
/// Called only by ilhook's trampoline, with the registers of the hooked method.
pub unsafe extern "win64" fn pump_detour(_regs: *mut Registers, _user_data: usize) {
    if !SLOT.has_job() {
        return;
    }
    IN_DETOUR.fetch_add(1, Ordering::SeqCst);
    let _ = catch_unwind(|| {
        if let Some(ops) = OPS.get() {
            // SAFETY: no arguments.
            let tid = unsafe { GetCurrentThreadId() };
            let main = tid == MAIN_TID.load(Ordering::Acquire);
            if pump_once(&SLOT, ops, main) == PumpResult::Panicked {
                // The job was already completed; nothing else to do in the game's thread.
            }
        }
    });
    IN_DETOUR.fetch_sub(1, Ordering::SeqCst);
}

/// The queue the worker's `SendService` uses: the slot, waited on with the real clock.
pub struct SlotQueue;

impl SendQueue for SlotQueue {
    fn submit(&mut self, frame: &[u8]) -> SendOutcome {
        match SLOT.push(frame.to_vec()) {
            Ok(()) => SLOT.wait_for_result(&RealClock),
            Err(outcome) => outcome,
        }
    }
}
