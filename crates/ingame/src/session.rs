//! The DLL's side of the pipe protocol: one request line in, one reply and what to do
//! next out. Pure: the probe, the sender and the log are passed in.

use crate::error::ProbeFailure;
use crate::log::LogSink;
use crate::send::Sender;
use resonance_types::ingame_pipe::{
    decode_request, encode_reply, ErrorCode, Reply, Request, PIPE_PROTOCOL_VERSION,
};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The walk to the live `ZTcpClient`; run at `hello` only, never on a timer (each `send`
/// walks again on the game's main thread, inside the pump).
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
        sender: &mut dyn Sender,
        log: &mut dyn LogSink,
    ) -> Outcome {
        let request = match decode_request(line) {
            Ok(r) => r,
            Err(e) => return error(e.id, e.code, "unreadable request", Next::Continue),
        };
        let id = request.id();
        match catch_unwind(AssertUnwindSafe(|| {
            self.dispatch(request, probe, sender, log)
        })) {
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
        sender: &mut dyn Sender,
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
            Request::Send { channel, text, .. } => {
                // Channel and length only: chat text is never logged.
                let shown = request.send_channel().map_or("?", |c| c.as_str());
                let note = format!("send channel={shown} text_len={}", text.len());
                let answer = sender.send(id, channel, text);
                match &answer {
                    Reply::Error { code, detail, .. } => {
                        log.info(&format!("{note}: {}: {detail}", code.as_str()));
                    }
                    _ => log.info(&format!("{note}: sent")),
                }
                reply(answer, Next::Continue)
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
    use crate::send::Sender;
    use resonance_types::ingame_pipe::{decode_reply, ErrorCode, Reply};

    struct FakeProbe {
        calls: usize,
        result: Result<(), ProbeFailure>,
        panics: bool,
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
    struct FakeSender {
        calls: Vec<(u32, String, String)>,
        panics: bool,
    }

    impl Sender for FakeSender {
        fn send(&mut self, id: u32, channel: &str, text: &str) -> Reply {
            self.calls.push((id, channel.into(), text.into()));
            if self.panics {
                panic!("sender panic");
            }
            Reply::Send {
                id,
                call_id: 0x8000_0007,
            }
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

    struct Rig {
        session: Session,
        probe: FakeProbe,
        sender: FakeSender,
        log: FakeLog,
    }

    impl Rig {
        fn new() -> Self {
            Self::with_probe(Ok(()))
        }
        fn with_probe(result: Result<(), ProbeFailure>) -> Self {
            Self {
                session: Session::new(),
                probe: FakeProbe {
                    calls: 0,
                    result,
                    panics: false,
                },
                sender: FakeSender::default(),
                log: FakeLog::default(),
            }
        }
        fn line(&mut self, line: &[u8]) -> Outcome {
            self.session
                .handle_line(line, &mut self.probe, &mut self.sender, &mut self.log)
        }
        fn hello(&mut self) -> Outcome {
            self.line(br#"{"op":"hello","id":1,"version":1}"#)
        }
        fn send(&mut self, channel: &str, text: &str) -> Outcome {
            let line = format!(r#"{{"op":"send","id":2,"channel":"{channel}","text":"{text}"}}"#);
            self.line(line.as_bytes())
        }
    }

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

    #[test]
    fn hello_runs_the_probe_once_even_when_it_fails() {
        for result in [Ok(()), Err(ProbeFailure::NoClient("singleton null".into()))] {
            let mut r = Rig::with_probe(result);
            let o = r.hello();
            assert_eq!(
                reply(&o),
                Reply::Hello {
                    id: 1,
                    version: 1,
                    dll: VERSION.into()
                }
            );
            assert_eq!(o.next, Next::Continue);
            assert_eq!(r.probe.calls, 1);
        }
    }

    #[test]
    fn hello_logs_a_failed_probe() {
        let mut r = Rig::with_probe(Err(ProbeFailure::Unsupported(StepError::method("Send"))));
        r.hello();
        assert!(r.log.0.iter().any(|m| m.contains("method_missing:Send")));
    }

    #[test]
    fn ping_needs_hello_first() {
        let mut r = Rig::new();
        let o = r.line(br#"{"op":"ping","id":8}"#);
        assert_eq!(reply(&o), err(8, ErrorCode::BadRequest, "hello first"));
        assert_eq!(o.next, Next::Continue);
        r.hello();
        let o = r.line(br#"{"op":"ping","id":9}"#);
        assert_eq!(reply(&o), Reply::Ack { id: 9 });
    }

    #[test]
    fn version_mismatch() {
        let mut r = Rig::new();
        let o = r.line(br#"{"op":"hello","id":1,"version":2}"#);
        assert_eq!(
            reply(&o),
            err(1, ErrorCode::Version, "dll speaks 1, app sent 2")
        );
        assert_eq!(o.next, Next::Continue);
    }

    #[test]
    fn send_goes_to_the_sender_and_its_reply_is_written() {
        let mut r = Rig::new();
        r.hello();
        let o = r.send("world", "hi");
        assert_eq!(
            reply(&o),
            Reply::Send {
                id: 2,
                call_id: 0x8000_0007
            }
        );
        assert_eq!(o.next, Next::Continue);
        assert_eq!(r.sender.calls, vec![(2, "world".into(), "hi".into())]);
        // The probe ran at hello only.
        assert_eq!(r.probe.calls, 1);
    }

    #[test]
    fn send_before_hello_never_reaches_the_sender() {
        let mut r = Rig::new();
        let o = r.send("world", "hi");
        assert_eq!(reply(&o), err(2, ErrorCode::BadRequest, "hello first"));
        assert!(r.sender.calls.is_empty());
    }

    #[test]
    fn chat_text_is_never_logged() {
        let mut r = Rig::new();
        r.hello();
        r.send("world", "secret text");
        assert!(r.log.0.iter().all(|m| !m.contains("secret")));
        assert!(r
            .log
            .0
            .iter()
            .any(|m| m.contains("world") && m.contains("11")));
    }

    #[test]
    fn a_panicking_sender_is_an_error_reply_and_the_session_lives() {
        let mut r = Rig::new();
        r.hello();
        r.sender.panics = true;
        let o = r.send("world", "hi");
        assert_eq!(reply(&o), err(2, ErrorCode::Unsupported, "internal error"));
        assert_eq!(o.next, Next::Continue);
        let o = r.line(br#"{"op":"ping","id":3}"#);
        assert_eq!(reply(&o), Reply::Ack { id: 3 });
    }

    #[test]
    fn a_panicking_probe_at_hello_is_an_error_reply() {
        let mut r = Rig::new();
        r.probe.panics = true;
        let o = r.hello();
        assert_eq!(reply(&o), err(1, ErrorCode::Unsupported, "internal error"));
        let o = r.line(br#"{"op":"ping","id":3}"#);
        assert_eq!(reply(&o), Reply::Ack { id: 3 });
    }

    #[test]
    fn bye_unloads_even_after_a_send() {
        let mut r = Rig::new();
        r.hello();
        r.send("say", "hi");
        let o = r.line(br#"{"op":"bye","id":4}"#);
        assert_eq!(reply(&o), Reply::Ack { id: 4 });
        assert_eq!(o.next, Next::Unload);
    }

    fn code_of(o: &Outcome) -> (u32, ErrorCode) {
        match reply(o) {
            Reply::Error { id, code, .. } => (id, code),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bad_lines() {
        let mut r = Rig::new();
        let o = r.line(br#"{"op":"nope","id":5}"#);
        assert_eq!(code_of(&o), (5, ErrorCode::BadRequest));
        assert_eq!(o.next, Next::Continue);
        let o = r.line(&[0xff, 0xfe]);
        assert_eq!(code_of(&o), (0, ErrorCode::BadRequest));
        assert_eq!(o.next, Next::Continue);
        let o = r.session.line_too_long(&mut r.log);
        assert_eq!(code_of(&o), (0, ErrorCode::BadRequest));
        assert_eq!(o.next, Next::Close);
    }
}
