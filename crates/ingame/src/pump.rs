//! What the hook's detour does on the game's main thread: take the job, walk to the live
//! `ZTcpClient` again, call `Send`, hand the result back. It never lets a panic out (the
//! detour is an `extern "win64"` function: unwinding through it would abort the game).

use crate::send::SendOutcome;
use crate::slot::SendSlot;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, TryLockError};

/// What the pump needs from the game.
pub trait SendOps {
    /// Walks from the singleton to the live client, now. Objects are never cached: a client
    /// from before a reconnect must not be called.
    fn live_client(&mut self) -> Result<usize, SendOutcome>;
    /// Calls `ZTcpClient.Send` on `client` with `frame`. Its return value is ignored.
    fn send(&mut self, client: usize, frame: &[u8]) -> Result<(), SendOutcome>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PumpResult {
    /// Nothing to do (or not the main thread).
    Idle,
    /// A job was taken and completed.
    Handled,
    /// A job is waiting but the ops were busy; the next tick takes it.
    Deferred,
    /// The work panicked: the job was completed `unsupported` and the slot is disabled.
    Panicked,
}

/// One tick of the pump. `main_thread` is the detour's `GetCurrentThreadId() == main_tid`.
pub fn pump_once<O: SendOps>(slot: &SendSlot, ops: &Mutex<O>, main_thread: bool) -> PumpResult {
    if !main_thread || !slot.has_job() {
        return PumpResult::Idle;
    }
    let mut ops = match ops.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return PumpResult::Deferred,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
    };
    let Some(frame) = slot.take_for_pump() else {
        return PumpResult::Idle;
    };
    let work = catch_unwind(AssertUnwindSafe(|| {
        let client = ops.live_client()?;
        ops.send(client, &frame)
    }));
    match work {
        Ok(Ok(())) => {
            slot.complete(SendOutcome::Sent);
            PumpResult::Handled
        }
        Ok(Err(outcome)) => {
            slot.complete(outcome);
            PumpResult::Handled
        }
        Err(_) => {
            slot.disable();
            slot.complete(SendOutcome::Unsupported("internal error".into()));
            PumpResult::Panicked
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::SendOutcome;
    use crate::slot::{Clock, SendSlot};
    use resonance_frame::{build_send_chat, WireChannel};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeSendOps {
        clients: VecDeque<Result<usize, SendOutcome>>,
        send_result: Option<Result<(), SendOutcome>>,
        send_panics: bool,
        forbidden: bool,
        walks: usize,
        sent: Vec<(usize, Vec<u8>)>,
    }

    impl FakeSendOps {
        fn with_client() -> Self {
            Self {
                clients: VecDeque::from([Ok(0x6000), Ok(0x6000)]),
                ..Default::default()
            }
        }
    }

    impl SendOps for FakeSendOps {
        fn live_client(&mut self) -> Result<usize, SendOutcome> {
            assert!(!self.forbidden, "the ops were touched");
            self.walks += 1;
            self.clients.pop_front().unwrap_or(Ok(0x6000))
        }
        fn send(&mut self, client: usize, frame: &[u8]) -> Result<(), SendOutcome> {
            assert!(!self.forbidden, "the ops were touched");
            if self.send_panics {
                panic!("send panic");
            }
            self.sent.push((client, frame.to_vec()));
            self.send_result.clone().unwrap_or(Ok(()))
        }
    }

    struct Instant;
    impl Clock for Instant {
        fn now_ms(&self) -> u64 {
            0
        }
        fn sleep_ms(&self, _: u64) {}
    }

    fn g2() -> Vec<u8> {
        build_send_chat(WireChannel::Say, "こんにちは", 0x8000_0001).unwrap()
    }

    fn outcome(slot: &SendSlot) -> SendOutcome {
        slot.wait_for_result(&Instant)
    }

    #[test]
    fn an_idle_slot_never_touches_the_ops() {
        let slot = SendSlot::new();
        let ops = Mutex::new(FakeSendOps {
            forbidden: true,
            ..Default::default()
        });
        assert_eq!(pump_once(&slot, &ops, true), PumpResult::Idle);
    }

    #[test]
    fn a_job_is_walked_then_sent_with_the_exact_frame() {
        let slot = SendSlot::new();
        let ops = Mutex::new(FakeSendOps::with_client());
        slot.push(g2()).unwrap();
        assert_eq!(pump_once(&slot, &ops, true), PumpResult::Handled);
        assert_eq!(outcome(&slot), SendOutcome::Sent);
        let ops = ops.lock().unwrap();
        assert_eq!(ops.walks, 1);
        assert_eq!(ops.sent, vec![(0x6000, g2())]);
    }

    #[test]
    fn a_walk_that_fails_sends_nothing() {
        for failure in [
            SendOutcome::NoClient("udp".into()),
            SendOutcome::Unsupported("method_missing:Send".into()),
        ] {
            let slot = SendSlot::new();
            let ops = Mutex::new(FakeSendOps {
                clients: VecDeque::from([Err(failure.clone())]),
                ..Default::default()
            });
            slot.push(g2()).unwrap();
            pump_once(&slot, &ops, true);
            assert_eq!(outcome(&slot), failure);
            assert!(ops.lock().unwrap().sent.is_empty());
        }
    }

    #[test]
    fn a_failed_send_completes_the_job() {
        let slot = SendSlot::new();
        let ops = Mutex::new(FakeSendOps {
            send_result: Some(Err(SendOutcome::NotSent("exception:Send".into()))),
            ..FakeSendOps::with_client()
        });
        slot.push(g2()).unwrap();
        pump_once(&slot, &ops, true);
        assert_eq!(
            outcome(&slot),
            SendOutcome::NotSent("exception:Send".into())
        );
    }

    #[test]
    fn every_job_is_walked_again() {
        let slot = SendSlot::new();
        let ops = Mutex::new(FakeSendOps {
            clients: VecDeque::from([Ok(0x6000), Err(SendOutcome::NoClient("handle zero".into()))]),
            ..Default::default()
        });
        slot.push(g2()).unwrap();
        pump_once(&slot, &ops, true);
        assert_eq!(outcome(&slot), SendOutcome::Sent);
        slot.push(g2()).unwrap();
        pump_once(&slot, &ops, true);
        assert_eq!(outcome(&slot), SendOutcome::NoClient("handle zero".into()));
        let ops = ops.lock().unwrap();
        assert_eq!((ops.walks, ops.sent.len()), (2, 1));
    }

    #[test]
    fn another_thread_leaves_the_job_filled() {
        let slot = SendSlot::new();
        let ops = Mutex::new(FakeSendOps {
            forbidden: true,
            ..Default::default()
        });
        slot.push(g2()).unwrap();
        assert_eq!(pump_once(&slot, &ops, false), PumpResult::Idle);
        assert!(slot.has_job());
        ops.lock().unwrap().forbidden = false;
        assert_eq!(pump_once(&slot, &ops, true), PumpResult::Handled);
        assert_eq!(outcome(&slot), SendOutcome::Sent);
    }

    #[test]
    fn a_busy_ops_mutex_defers_the_job() {
        let slot = SendSlot::new();
        let ops = Mutex::new(FakeSendOps::with_client());
        slot.push(g2()).unwrap();
        {
            let _held = ops.lock().unwrap();
            assert_eq!(pump_once(&slot, &ops, true), PumpResult::Deferred);
            assert!(slot.has_job());
        }
        assert_eq!(pump_once(&slot, &ops, true), PumpResult::Handled);
        assert_eq!(outcome(&slot), SendOutcome::Sent);
    }

    #[test]
    fn a_panic_disables_the_pump_and_stays_inside() {
        let slot = SendSlot::new();
        let ops = Mutex::new(FakeSendOps {
            send_panics: true,
            ..FakeSendOps::with_client()
        });
        slot.push(g2()).unwrap();
        assert_eq!(pump_once(&slot, &ops, true), PumpResult::Panicked);
        assert_eq!(
            outcome(&slot),
            SendOutcome::Unsupported("internal error".into())
        );
        assert_eq!(slot.push(g2()), Err(SendOutcome::Disabled));
        // The poisoned mutex is not touched again.
        assert_eq!(pump_once(&slot, &ops, true), PumpResult::Idle);
    }
}
