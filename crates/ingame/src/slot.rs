#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::SendOutcome;
    use std::cell::{Cell, RefCell};

    /// A clock whose `sleep_ms` advances time and runs the actions scheduled up to it.
    pub(crate) struct FakeClock<'a> {
        now: Cell<u64>,
        script: RefCell<Vec<(u64, Box<dyn FnMut() + 'a>)>>,
    }

    impl<'a> FakeClock<'a> {
        fn new() -> Self {
            Self {
                now: Cell::new(0),
                script: RefCell::new(Vec::new()),
            }
        }
        fn at(self, time: u64, action: impl FnMut() + 'a) -> Self {
            self.script.borrow_mut().push((time, Box::new(action)));
            self
        }
        fn now(&self) -> u64 {
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
