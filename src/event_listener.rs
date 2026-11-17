//! A backend event listener that ends with its owner. `listen` hands back the function that
//! removes the listener; `Listener` keeps it and calls it on a restart and on cleanup, so no
//! closure is leaked with `forget()`.

use crate::tauri_bridge::listen;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

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
        if self.closed {
            return None;
        }
        self.generation += 1;
        Some((self.generation, self.live.take()))
    }

    /// Stores the value a registration resolved to. A value that arrives after a close or after
    /// a newer `begin` comes back for the caller to drop at once.
    pub fn attach(&mut self, token: u64, value: T) -> Option<T> {
        if self.closed || token != self.generation {
            return Some(value);
        }
        self.live = Some(value);
        None
    }

    /// The owner is gone: returns the live value (once), and refuses every later `begin`/`attach`.
    pub fn close(&mut self) -> Option<T> {
        self.closed = true;
        self.generation += 1;
        self.live.take()
    }
}

/// One registered listener: the function that removes it, and the handler it calls. Dropping it
/// unlistens.
struct Active {
    unlisten: Option<js_sys::Function>,
    closure: Option<Closure<dyn FnMut(JsValue)>>,
}

impl Active {
    fn new(handle: JsValue, closure: Closure<dyn FnMut(JsValue)>) -> Self {
        Self {
            unlisten: handle.dyn_into::<js_sys::Function>().ok(),
            closure: Some(closure),
        }
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        match &self.unlisten {
            Some(unlisten) => {
                let _ = unlisten.call0(&JsValue::UNDEFINED);
            }
            // `listen` gave no function to remove it with: JS may still hold the handler, which
            // must stay callable (the old behaviour for this case).
            None => {
                if let Some(closure) = self.closure.take() {
                    closure.forget();
                }
            }
        }
    }
}

/// One backend event listener owned by the component (or function) that made it. Make it with
/// `Listener::new()` where an owner is current; `start` registers (replacing the previous
/// registration); cleanup of the owner removes it, even when `listen` resolves after that.
#[derive(Clone, Copy)]
pub struct Listener(StoredValue<Slot<Active>, LocalStorage>);

impl Listener {
    pub fn new() -> Self {
        let slot = StoredValue::new_local(Slot::<Active>::default());
        on_cleanup(move || {
            // Dropped outside the slot's borrow: `Active::drop` calls into JS.
            let live = slot.try_update_value(|s| s.close()).flatten();
            drop(live);
        });
        Self(slot)
    }

    /// Listens to `event` with `handler`, removing the listener this one started before. Does
    /// nothing once the owner is gone.
    pub fn start(self, event: &'static str, handler: impl FnMut(JsValue) + 'static) {
        let Some(Some((token, old))) = self.0.try_update_value(|s| s.begin()) else {
            return;
        };
        drop(old);
        let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(JsValue)>);
        let slot = self.0;
        spawn_local(async move {
            let handle = listen(event, &closure).await;
            let active = Active::new(handle, closure);
            // A disposed slot or a stale token: the value comes back (or, with the slot gone,
            // is dropped with the closure), and its `Drop` unlistens.
            if let Some(Some(stale)) = slot.try_update_value(|s| s.attach(token, active)) {
                drop(stale);
            }
        });
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
