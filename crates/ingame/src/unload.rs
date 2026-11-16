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
