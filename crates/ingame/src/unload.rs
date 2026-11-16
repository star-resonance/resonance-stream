//! The unload sequence, in the order the design doc fixes. Each step is guarded, so one
//! that panics does not leave the rest undone; the sequence runs once.

use std::panic::{catch_unwind, AssertUnwindSafe};

/// What the real DLL does at each step (`win/entry.rs`).
pub trait UnloadSteps {
    /// Stop taking requests.
    fn stop_accepting(&mut self);
    /// Remove the hooks (nothing to remove until R5).
    fn disable_hooks(&mut self);
    /// Let work in flight finish (nothing in flight until R5).
    fn drain(&mut self);
    fn close_pipe(&mut self);
    fn detach_thread(&mut self);
    fn close_log(&mut self);
}

#[derive(Default)]
pub struct Unload {
    done: bool,
}

impl Unload {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs the steps, once; later calls do nothing.
    pub fn run(&mut self, steps: &mut dyn UnloadSteps) {
        if std::mem::replace(&mut self.done, true) {
            return;
        }
        let guarded = |f: &mut dyn FnMut()| {
            let _ = catch_unwind(AssertUnwindSafe(f));
        };
        guarded(&mut || steps.stop_accepting());
        guarded(&mut || steps.disable_hooks());
        guarded(&mut || steps.drain());
        guarded(&mut || steps.close_pipe());
        guarded(&mut || steps.detach_thread());
        guarded(&mut || steps.close_log());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        calls: Vec<&'static str>,
        panic_on: Option<&'static str>,
    }

    impl Recorder {
        fn step(&mut self, name: &'static str) {
            self.calls.push(name);
            if self.panic_on == Some(name) {
                panic!("boom in {name}");
            }
        }
    }

    impl UnloadSteps for Recorder {
        fn stop_accepting(&mut self) {
            self.step("stop_accepting");
        }
        fn disable_hooks(&mut self) {
            self.step("disable_hooks");
        }
        fn drain(&mut self) {
            self.step("drain");
        }
        fn close_pipe(&mut self) {
            self.step("close_pipe");
        }
        fn detach_thread(&mut self) {
            self.step("detach_thread");
        }
        fn close_log(&mut self) {
            self.step("close_log");
        }
    }

    const ORDER: [&str; 6] = [
        "stop_accepting",
        "disable_hooks",
        "drain",
        "close_pipe",
        "detach_thread",
        "close_log",
    ];

    #[test]
    fn runs_in_order() {
        let mut r = Recorder::default();
        Unload::new().run(&mut r);
        assert_eq!(r.calls, ORDER);
    }

    #[test]
    fn a_panicking_step_does_not_stop_the_rest() {
        let mut r = Recorder {
            panic_on: Some("drain"),
            ..Default::default()
        };
        Unload::new().run(&mut r);
        assert_eq!(r.calls, ORDER);
    }

    #[test]
    fn runs_once() {
        let mut r = Recorder::default();
        let mut u = Unload::new();
        u.run(&mut r);
        u.run(&mut r);
        assert_eq!(r.calls, ORDER);
    }
}
