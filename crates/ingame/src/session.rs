#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ProbeFailure, StepError};
    use crate::log::{Level, LogSink};
    use resonance_types::ingame_pipe::{decode_reply, ErrorCode, Reply};

    struct FakeProbe {
        calls: usize,
        result: Result<(), ProbeFailure>,
        panics: bool,
    }

    impl FakeProbe {
        fn ok() -> Self {
            Self::with(Ok(()))
        }
        fn with(result: Result<(), ProbeFailure>) -> Self {
            Self {
                calls: 0,
                result,
                panics: false,
            }
        }
    }

    impl Probe for FakeProbe {
        fn probe(&mut self) -> Result<(), ProbeFailure> {
            self.calls += 1;
            if self.panics {
                panic!("probe panic");
            }
            self.result.clone()
        }
    }

    #[derive(Default)]
    struct FakeLog(Vec<String>);

    impl LogSink for FakeLog {
        fn log(&mut self, _level: Level, message: &str) {
            self.0.push(message.to_string());
        }
    }

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    fn reply(o: &Outcome) -> Reply {
        decode_reply(o.reply.strip_suffix(b"\n").expect("one line")).unwrap()
    }

    fn err(id: u32, code: ErrorCode, detail: &str) -> Reply {
        Reply::Error {
            id,
            code,
            detail: detail.into(),
        }
    }

    fn hello(s: &mut Session, p: &mut FakeProbe, l: &mut FakeLog) -> Outcome {
        s.handle_line(br#"{"op":"hello","id":1,"version":1}"#, p, l)
    }

    #[test]
    fn hello_runs_the_probe_once_even_when_it_fails() {
        for result in [Ok(()), Err(ProbeFailure::NoClient("singleton null".into()))] {
            let (mut s, mut p, mut l) =
                (Session::new(), FakeProbe::with(result), FakeLog::default());
            let o = hello(&mut s, &mut p, &mut l);
            assert_eq!(
                reply(&o),
                Reply::Hello {
                    id: 1,
                    version: 1,
                    dll: VERSION.into()
                }
            );
            assert_eq!(o.next, Next::Continue);
            assert_eq!(p.calls, 1);
        }
    }

    #[test]
    fn ping_needs_hello_first() {
        let (mut s, mut p, mut l) = (Session::new(), FakeProbe::ok(), FakeLog::default());
        let o = s.handle_line(br#"{"op":"ping","id":8}"#, &mut p, &mut l);
        assert_eq!(reply(&o), err(8, ErrorCode::BadRequest, "hello first"));
        assert_eq!(o.next, Next::Continue);
        hello(&mut s, &mut p, &mut l);
        let o = s.handle_line(br#"{"op":"ping","id":9}"#, &mut p, &mut l);
        assert_eq!(reply(&o), Reply::Ack { id: 9 });
    }

    #[test]
    fn version_mismatch() {
        let (mut s, mut p, mut l) = (Session::new(), FakeProbe::ok(), FakeLog::default());
        let o = s.handle_line(br#"{"op":"hello","id":1,"version":2}"#, &mut p, &mut l);
        assert_eq!(
            reply(&o),
            err(1, ErrorCode::Version, "dll speaks 1, app sent 2")
        );
        assert_eq!(o.next, Next::Continue);
    }

    fn send(s: &mut Session, p: &mut FakeProbe, l: &mut FakeLog, channel: &str) -> Outcome {
        let line = format!(r#"{{"op":"send","id":2,"channel":"{channel}","text":"secret text"}}"#);
        s.handle_line(line.as_bytes(), p, l)
    }

    #[test]
    fn send_checks_the_channel_before_the_probe() {
        let (mut s, mut p, mut l) = (Session::new(), FakeProbe::ok(), FakeLog::default());
        hello(&mut s, &mut p, &mut l);
        let before = p.calls;
        let o = send(&mut s, &mut p, &mut l, "whisper");
        assert_eq!(code_of(&o), (2, ErrorCode::BadChannel));
        assert_eq!(p.calls, before);
    }

    fn code_of(o: &Outcome) -> (u32, ErrorCode) {
        match reply(o) {
            Reply::Error { id, code, .. } => (id, code),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn send_answers_from_the_probe() {
        let cases = [
            (
                Err(ProbeFailure::NoClient("singleton null".into())),
                err(2, ErrorCode::NoClient, "singleton null"),
            ),
            (
                Err(ProbeFailure::Unsupported(StepError::method("GetTransport"))),
                err(2, ErrorCode::Unsupported, "method_missing:GetTransport"),
            ),
            (
                Ok(()),
                err(2, ErrorCode::Unsupported, "send not built (R4)"),
            ),
        ];
        for (result, expected) in cases {
            let (mut s, mut p, mut l) =
                (Session::new(), FakeProbe::with(result), FakeLog::default());
            hello(&mut s, &mut p, &mut l);
            let before = p.calls;
            let o = send(&mut s, &mut p, &mut l, "world");
            assert_eq!(reply(&o), expected);
            assert_eq!(o.next, Next::Continue);
            assert_eq!(p.calls, before + 1);
        }
    }

    #[test]
    fn bye_unloads() {
        let (mut s, mut p, mut l) = (Session::new(), FakeProbe::ok(), FakeLog::default());
        let o = s.handle_line(br#"{"op":"bye","id":4}"#, &mut p, &mut l);
        assert_eq!(reply(&o), Reply::Ack { id: 4 });
        assert_eq!(o.next, Next::Unload);
    }

    #[test]
    fn bad_lines() {
        let (mut s, mut p, mut l) = (Session::new(), FakeProbe::ok(), FakeLog::default());
        let o = s.handle_line(br#"{"op":"nope","id":5}"#, &mut p, &mut l);
        assert_eq!(code_of(&o), (5, ErrorCode::BadRequest));
        assert_eq!(o.next, Next::Continue);
        let o = s.handle_line(&[0xff, 0xfe], &mut p, &mut l);
        assert_eq!(code_of(&o), (0, ErrorCode::BadRequest));
        assert_eq!(o.next, Next::Continue);
        let o = s.line_too_long(&mut l);
        assert_eq!(code_of(&o), (0, ErrorCode::BadRequest));
        assert_eq!(o.next, Next::Close);
    }

    #[test]
    fn a_panicking_probe_is_an_error_reply_and_the_session_lives() {
        let (mut s, mut p, mut l) = (Session::new(), FakeProbe::ok(), FakeLog::default());
        hello(&mut s, &mut p, &mut l);
        p.panics = true;
        let o = send(&mut s, &mut p, &mut l, "world");
        assert_eq!(reply(&o), err(2, ErrorCode::Unsupported, "internal error"));
        assert_eq!(o.next, Next::Continue);
        let o = s.handle_line(br#"{"op":"ping","id":3}"#, &mut p, &mut l);
        assert_eq!(reply(&o), Reply::Ack { id: 3 });
    }

    #[test]
    fn chat_text_is_never_replied_or_logged() {
        for result in [Ok(()), Err(ProbeFailure::NoClient("udp".into()))] {
            let (mut s, mut p, mut l) =
                (Session::new(), FakeProbe::with(result), FakeLog::default());
            hello(&mut s, &mut p, &mut l);
            let o = send(&mut s, &mut p, &mut l, "world");
            assert!(!String::from_utf8_lossy(&o.reply).contains("secret"));
            assert!(!l.0.is_empty());
            assert!(l.0.iter().all(|m| !m.contains("secret")));
            assert!(l.0.iter().any(|m| m.contains("world") && m.contains("11")));
        }
    }
}
