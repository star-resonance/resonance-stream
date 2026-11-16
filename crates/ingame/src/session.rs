//! The DLL's side of the pipe protocol: one request line in, one reply and what to do
//! next out. Pure: the probe and the log are passed in.

use crate::error::ProbeFailure;
use crate::log::LogSink;
use resonance_types::ingame_pipe::{
    decode_request, encode_reply, ErrorCode, Reply, Request, PIPE_PROTOCOL_VERSION,
};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The walk to the live `ZTcpClient`; run at `hello` and on each `send`, never on a timer.
pub trait Probe {
    fn probe(&mut self) -> Result<(), ProbeFailure>;
}

/// What the worker does after writing the reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Continue,
    /// Close the connection (and so unload).
    Close,
    /// `bye`: unload.
    Unload,
}

#[derive(Debug)]
pub struct Outcome {
    /// The reply line, `\n` included.
    pub reply: Vec<u8>,
    pub next: Next,
}

#[derive(Default)]
pub struct Session {
    greeted: bool,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one request line (without its `\n`).
    pub fn handle_line(
        &mut self,
        line: &[u8],
        probe: &mut dyn Probe,
        log: &mut dyn LogSink,
    ) -> Outcome {
        let request = match decode_request(line) {
            Ok(r) => r,
            Err(e) => return error(e.id, e.code, "unreadable request", Next::Continue),
        };
        let id = request.id();
        match catch_unwind(AssertUnwindSafe(|| self.dispatch(request, probe, log))) {
            Ok(outcome) => outcome,
            Err(_) => {
                log.warn("a request handler panicked");
                error(id, ErrorCode::Unsupported, "internal error", Next::Continue)
            }
        }
    }

    /// A line over the cap: `bad_request`, and the connection closes.
    pub fn line_too_long(&mut self, log: &mut dyn LogSink) -> Outcome {
        log.warn("request line over the cap; closing");
        error(0, ErrorCode::BadRequest, "line too long", Next::Close)
    }

    fn dispatch(
        &mut self,
        request: Request,
        probe: &mut dyn Probe,
        log: &mut dyn LogSink,
    ) -> Outcome {
        let id = request.id();
        if !self.greeted && !matches!(request, Request::Hello { .. } | Request::Bye { .. }) {
            return error(id, ErrorCode::BadRequest, "hello first", Next::Continue);
        }
        match &request {
            Request::Hello { version, .. } => {
                if *version != PIPE_PROTOCOL_VERSION {
                    log.warn(&format!("hello with version {version}"));
                    let detail = format!("dll speaks {PIPE_PROTOCOL_VERSION}, app sent {version}");
                    return error(id, ErrorCode::Version, &detail, Next::Continue);
                }
                self.greeted = true;
                match probe.probe() {
                    Ok(()) => log.info("hello: probe walked: ok"),
                    Err(f) => log.info(&format!("hello: probe failed: {}", f.detail())),
                }
                reply(
                    Reply::Hello {
                        id,
                        version: PIPE_PROTOCOL_VERSION,
                        dll: env!("CARGO_PKG_VERSION").into(),
                    },
                    Next::Continue,
                )
            }
            Request::Ping { .. } => reply(Reply::Ack { id }, Next::Continue),
            Request::Bye { .. } => {
                log.info("bye");
                reply(Reply::Ack { id }, Next::Unload)
            }
            Request::Send { text, .. } => {
                let channel = match request.send_channel() {
                    Ok(c) => c,
                    Err(code) => return error(id, code, "unknown channel", Next::Continue),
                };
                // Channel and length only: chat text is never logged.
                let note = format!("send channel={} text_len={}", channel.as_str(), text.len());
                match probe.probe() {
                    Err(f) => {
                        log.info(&format!("{note}: probe failed: {}", f.detail()));
                        error(id, f.error_code(), &f.detail(), Next::Continue)
                    }
                    Ok(()) => {
                        log.info(&format!("{note}: probe walked: ok; send not built"));
                        error(
                            id,
                            ErrorCode::Unsupported,
                            "send not built (R4)",
                            Next::Continue,
                        )
                    }
                }
            }
        }
    }
}

fn reply(reply: Reply, next: Next) -> Outcome {
    Outcome {
        reply: encode_reply(&reply).into_bytes(),
        next,
    }
}

fn error(id: u32, code: ErrorCode, detail: &str, next: Next) -> Outcome {
    reply(
        Reply::Error {
            id,
            code,
            detail: detail.into(),
        },
        next,
    )
}

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
