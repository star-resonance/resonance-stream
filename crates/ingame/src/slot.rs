//! The one-slot queue between the worker thread (which holds a request for at most the
//! DLL's hold) and the pump (which runs on the game's main thread). One state byte decides
//! every hand-over atomically; the pump's idle path is a single atomic load.

use crate::send::SendOutcome;
use resonance_types::ingame_pipe::DLL_HOLD_MS;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError};

/// Jobs the slot holds (doc: a single slot).
pub const SEND_QUEUE_CAP: usize = 1;
/// How often the worker looks at the slot.
pub const POLL_MS: u64 = 1;
/// How long the worker waits for a job the pump has already taken. The app gave up long
/// before (it waits 1500 ms).
pub const TAKEN_WAIT_CAP_MS: u64 = 5000;

/// Time, passed in so waits are tested without sleeping.
pub trait Clock {
    fn now_ms(&self) -> u64;
    fn sleep_ms(&self, ms: u64);
}

const EMPTY: u8 = 0;
const FILLED: u8 = 1;
const TAKEN: u8 = 2;
const DONE: u8 = 3;
/// The worker gave up on a taken job; the pump's `complete` frees the slot.
const ABANDONED: u8 = 4;

pub struct SendSlot {
    state: AtomicU8,
    closed: AtomicBool,
    job: Mutex<Option<Vec<u8>>>,
    result: Mutex<Option<SendOutcome>>,
}

impl Default for SendSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl SendSlot {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            closed: AtomicBool::new(false),
            job: Mutex::new(None),
            result: Mutex::new(None),
        }
    }

    /// Worker side: puts a frame in. `Busy` if the slot is in use, `Disabled` once closed.
    pub fn push(&self, frame: Vec<u8>) -> Result<(), SendOutcome> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SendOutcome::Disabled);
        }
        if self.state.load(Ordering::Acquire) != EMPTY {
            return Err(SendOutcome::Busy);
        }
        *self.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
        self.state.store(FILLED, Ordering::Release);
        Ok(())
    }

    /// Pump side, the idle path: one atomic load.
    pub fn has_job(&self) -> bool {
        self.state.load(Ordering::Acquire) == FILLED
    }

    /// Pump side: takes the job, unless the worker expired it first.
    pub fn take_for_pump(&self) -> Option<Vec<u8>> {
        self.state
            .compare_exchange(FILLED, TAKEN, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        self.job
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Worker side: withdraws a job the pump has not taken. `false` if it was taken.
    pub fn try_expire(&self) -> bool {
        if self
            .state
            .compare_exchange(FILLED, EMPTY, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        self.job
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        true
    }

    /// Pump side: finishes the job it took.
    pub fn complete(&self, outcome: SendOutcome) {
        *self.result.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
        if self
            .state
            .compare_exchange(TAKEN, DONE, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // The worker abandoned it: nobody reads the result, the slot is free.
            self.result
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            self.state.store(EMPTY, Ordering::Release);
        }
    }

    /// No new job is accepted (unload).
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// No new job is accepted (the pump panicked).
    pub fn disable(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Worker side: waits for the pump's answer to the job pushed just before. A job the
    /// pump has not taken within [`DLL_HOLD_MS`] is withdrawn (`Expired`: never sent).
    pub fn wait_for_result(&self, clock: &dyn Clock) -> SendOutcome {
        let start = clock.now_ms();
        loop {
            let elapsed = clock.now_ms().saturating_sub(start);
            match self.state.load(Ordering::Acquire) {
                DONE => {
                    let outcome = self
                        .result
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take()
                        .unwrap_or_else(|| SendOutcome::Unsupported("internal error".into()));
                    self.state.store(EMPTY, Ordering::Release);
                    return outcome;
                }
                FILLED if elapsed >= u64::from(DLL_HOLD_MS) => {
                    if self.try_expire() {
                        return SendOutcome::Expired;
                    }
                }
                TAKEN if elapsed >= TAKEN_WAIT_CAP_MS => {
                    if self
                        .state
                        .compare_exchange(TAKEN, ABANDONED, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        return SendOutcome::NotSent("pump stuck".into());
                    }
                }
                _ => {}
            }
            clock.sleep_ms(POLL_MS);
        }
    }
}

#[cfg(test)]
pub(crate) mod fake_clock {
    use super::Clock;
    use std::cell::{Cell, RefCell};

    /// A clock whose `sleep_ms` advances time and runs the actions scheduled up to it.
    pub(crate) struct FakeClock<'a> {
        now: Cell<u64>,
        script: RefCell<Vec<(u64, Box<dyn FnMut() + 'a>)>>,
    }

    impl<'a> FakeClock<'a> {
        pub(crate) fn new() -> Self {
            Self {
                now: Cell::new(0),
                script: RefCell::new(Vec::new()),
            }
        }
        pub(crate) fn at(self, time: u64, action: impl FnMut() + 'a) -> Self {
            self.script.borrow_mut().push((time, Box::new(action)));
            self
        }
        pub(crate) fn now(&self) -> u64 {
            self.now.get()
        }
    }

    impl Clock for FakeClock<'_> {
        fn now_ms(&self) -> u64 {
            self.now.get()
        }
        fn sleep_ms(&self, ms: u64) {
            self.now.set(self.now.get() + ms);
            let now = self.now.get();
            let mut script = self.script.borrow_mut();
            let mut i = 0;
            while i < script.len() {
                if script[i].0 <= now {
                    let (_, mut action) = script.remove(i);
                    action();
                } else {
                    i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake_clock::FakeClock;
    use super::*;
    use crate::send::SendOutcome;

    #[test]
    fn constants() {
        assert_eq!(SEND_QUEUE_CAP, 1);
        assert_eq!(POLL_MS, 1);
        assert_eq!(TAKEN_WAIT_CAP_MS, 5000);
    }

    #[test]
    fn an_empty_slot_has_nothing() {
        let slot = SendSlot::new();
        assert!(!slot.has_job());
        assert_eq!(slot.take_for_pump(), None);
    }

    #[test]
    fn one_job_at_a_time() {
        let slot = SendSlot::new();
        assert_eq!(slot.push(vec![1]), Ok(()));
        assert!(slot.has_job());
        assert_eq!(slot.push(vec![2]), Err(SendOutcome::Busy));
        assert_eq!(slot.take_for_pump(), Some(vec![1]));
        assert_eq!(slot.push(vec![3]), Err(SendOutcome::Busy));
    }

    #[test]
    fn a_closed_slot_takes_nothing() {
        let slot = SendSlot::new();
        slot.close();
        assert_eq!(slot.push(vec![1]), Err(SendOutcome::Disabled));
        let slot = SendSlot::new();
        slot.disable();
        assert_eq!(slot.push(vec![1]), Err(SendOutcome::Disabled));
    }

    #[test]
    fn expiry_is_decided_atomically() {
        let slot = SendSlot::new();
        slot.push(vec![1]).unwrap();
        assert!(slot.try_expire());
        assert_eq!(slot.take_for_pump(), None);

        let slot = SendSlot::new();
        slot.push(vec![1]).unwrap();
        assert!(slot.take_for_pump().is_some());
        assert!(!slot.try_expire());
    }

    #[test]
    fn nothing_takes_the_job_so_it_expires_at_the_hold() {
        let slot = SendSlot::new();
        slot.push(vec![1]).unwrap();
        let clock = FakeClock::new();
        assert_eq!(slot.wait_for_result(&clock), SendOutcome::Expired);
        assert_eq!(clock.now(), 1000);
        // The slot is free again and the expired job is gone.
        assert_eq!(slot.take_for_pump(), None);
        assert_eq!(slot.push(vec![2]), Ok(()));
    }

    #[test]
    fn a_job_taken_in_time_is_waited_for() {
        let slot = SendSlot::new();
        slot.push(vec![1]).unwrap();
        let clock = FakeClock::new()
            .at(999, || assert!(slot.take_for_pump().is_some()))
            .at(1004, || slot.complete(SendOutcome::Sent));
        assert_eq!(slot.wait_for_result(&clock), SendOutcome::Sent);
        assert_eq!(clock.now(), 1004);
        assert_eq!(slot.push(vec![2]), Ok(()));
    }

    #[test]
    fn a_taken_job_that_never_completes_gives_up_at_the_cap() {
        let slot = SendSlot::new();
        slot.push(vec![1]).unwrap();
        let clock = FakeClock::new().at(10, || assert!(slot.take_for_pump().is_some()));
        assert_eq!(
            slot.wait_for_result(&clock),
            SendOutcome::NotSent("pump stuck".into())
        );
        assert_eq!(clock.now(), 5000);
        // Still taken: nothing new goes in until the pump finishes, then it is free.
        assert_eq!(slot.push(vec![2]), Err(SendOutcome::Busy));
        slot.complete(SendOutcome::Sent);
        assert_eq!(slot.push(vec![2]), Ok(()));
    }

    #[test]
    fn the_slot_crosses_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SendSlot>();
    }
}
