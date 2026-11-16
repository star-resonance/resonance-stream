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
        fail: std::collections::HashMap<&'static str, String>,
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
        fn disable_hooks(&mut self) -> Result<(), String> {
            self.step("disable_hooks");
            self.fail.get("disable_hooks").cloned().map_or(Ok(()), Err)
        }
        fn drain(&mut self) -> Result<(), String> {
            self.step("drain");
            self.fail.get("drain").cloned().map_or(Ok(()), Err)
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

    #[test]
    fn all_steps_ok_unloads() {
        let mut r = Recorder::default();
        assert_eq!(Unload::new().run(&mut r), UnloadOutcome::Unloaded);
    }

    #[test]
    fn a_failed_unhook_keeps_the_dll_resident() {
        let mut r = Recorder::default();
        r.fail.insert("disable_hooks", "still patched".into());
        assert_eq!(Unload::new().run(&mut r), UnloadOutcome::KeepResident);
        assert_eq!(
            r.calls,
            [
                "stop_accepting",
                "disable_hooks",
                "close_pipe",
                "detach_thread",
                "close_log"
            ]
        );
    }

    #[test]
    fn a_panicking_unhook_keeps_the_dll_resident() {
        let mut r = Recorder {
            panic_on: Some("disable_hooks"),
            ..Default::default()
        };
        assert_eq!(Unload::new().run(&mut r), UnloadOutcome::KeepResident);
        assert!(!r.calls.contains(&"drain"));
        assert!(r.calls.contains(&"close_log"));
    }

    #[test]
    fn a_stuck_drain_keeps_the_dll_resident() {
        let mut r = Recorder::default();
        r.fail.insert("drain", "detour still running".into());
        assert_eq!(Unload::new().run(&mut r), UnloadOutcome::KeepResident);
        assert_eq!(r.calls, ORDER);
    }

    #[test]
    fn a_second_run_changes_nothing_and_repeats_the_outcome() {
        let mut r = Recorder::default();
        r.fail.insert("disable_hooks", "x".into());
        let mut u = Unload::new();
        assert_eq!(u.run(&mut r), UnloadOutcome::KeepResident);
        let calls = r.calls.len();
        assert_eq!(u.run(&mut r), UnloadOutcome::KeepResident);
        assert_eq!(r.calls.len(), calls);
    }
}
