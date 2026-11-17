//! A backend event listener that ends with its owner. `listen` hands back the function that
//! removes the listener; `Listener` keeps it and calls it on a restart and on cleanup, so no
//! closure is leaked with `forget()`.

/// The pure state of one listener: at most one live (or pending) registration, a generation that
/// tells a stale registration from the current one, and whether the owner is gone. `T` is what
/// must be released (dropping it unlistens).
pub struct Slot<T> {
    generation: u64,
    live: Option<T>,
    closed: bool,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            generation: 0,
            live: None,
            closed: false,
        }
    }
}

impl<T> Slot<T> {
    /// Starts a registration: the new token, and the previously live value for the caller to
    /// drop. `None` when the owner is gone.
    pub fn begin(&mut self) -> Option<(u64, Option<T>)> {
        None
    }

    /// Stores the value a registration resolved to. A value that arrives after a close or after
    /// a newer `begin` comes back for the caller to drop at once.
    pub fn attach(&mut self, _token: u64, value: T) -> Option<T> {
        Some(value)
    }

    /// The owner is gone: returns the live value (once), and refuses every later `begin`/`attach`.
    pub fn close(&mut self) -> Option<T> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn fresh_slot_attach_then_close_returns_the_value_once() {
        let mut slot = Slot::<&'static str>::default();
        assert_eq!(slot.begin(), Some((1, None)));
        assert_eq!(slot.attach(1, "a"), None);
        assert_eq!(slot.close(), Some("a"));
        assert_eq!(slot.close(), None);
    }

    #[test]
    fn restart_hands_back_the_old_value_and_never_holds_two() {
        let mut slot = Slot::<&'static str>::default();
        assert_eq!(slot.begin(), Some((1, None)));
        assert_eq!(slot.attach(1, "a"), None);
        assert_eq!(slot.begin(), Some((2, Some("a"))));
        assert_eq!(slot.attach(2, "b"), None);
        assert_eq!(slot.close(), Some("b"));
    }

    #[test]
    fn stale_registration_is_returned_at_once() {
        let mut slot = Slot::<&'static str>::default();
        assert_eq!(slot.begin(), Some((1, None)));
        assert_eq!(slot.begin(), Some((2, None)));
        assert_eq!(slot.attach(1, "a"), Some("a"));
        assert_eq!(slot.attach(2, "b"), None);
        assert_eq!(slot.close(), Some("b"));
    }

    #[test]
    fn registration_after_close_is_returned_and_begin_refuses() {
        let mut slot = Slot::<&'static str>::default();
        assert_eq!(slot.begin(), Some((1, None)));
        assert_eq!(slot.close(), None);
        assert_eq!(slot.attach(1, "a"), Some("a"));
        assert_eq!(slot.begin(), None);
    }

    /// Counts itself in a shared cell while alive.
    struct Counted(Rc<Cell<i32>>);
    impl Counted {
        fn new(count: &Rc<Cell<i32>>) -> Self {
            count.set(count.get() + 1);
            Self(count.clone())
        }
    }
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }

    #[test]
    fn mount_unmount_mount_keeps_the_live_count_at_one() {
        let count = Rc::new(Cell::new(0));
        for _ in 0..2 {
            let mut slot = Slot::<Counted>::default();
            let (t, old) = slot.begin().unwrap();
            drop(old);
            assert!(slot.attach(t, Counted::new(&count)).is_none());
            assert_eq!(count.get(), 1);
            // A restart: the old one is dropped before the new one exists.
            let (t, old) = slot.begin().unwrap();
            drop(old);
            assert_eq!(count.get(), 0);
            assert!(slot.attach(t, Counted::new(&count)).is_none());
            assert_eq!(count.get(), 1);
            drop(slot.close());
            assert_eq!(count.get(), 0);
        }
    }
}
