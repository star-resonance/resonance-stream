//! The wire format between the app and the in-game DLL (`resonance_ingame.dll`) on the
//! named pipe, and the pure rules both ends share. JSON lines, UTF-8, `\n`-terminated,
//! at most [`MAX_LINE_BYTES`] bytes each; the app sends requests, the DLL answers each
//! with one reply that echoes the request's `id`. Korean sentences are not here: the app
//! maps each [`ErrorCode`] to its own.

use crate::SendChatChannel;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The pipe protocol's version, sent in `hello`.
pub const PIPE_PROTOCOL_VERSION: u32 = 1;
/// The longest line, not counting its `\n`.
pub const MAX_LINE_BYTES: usize = 4096;
/// The longest `detail` of an error reply, in bytes.
pub const MAX_DETAIL_BYTES: usize = 256;
/// How long the app waits for a request write to the pipe.
pub const WRITE_TIMEOUT_MS: u32 = 100;
/// How long the DLL holds a send request before it expires.
pub const DLL_HOLD_MS: u32 = 1000;
/// How long the app waits for a reply.
pub const APP_WAIT_MS: u32 = 1500;

/// The pipe the app creates for the game process `pid`.
pub fn pipe_name(pid: u32) -> String {
    format!(r"\\.\pipe\resonance-ingame-{pid}")
}

/// Why a request failed; the app picks the user's sentence by this, never by `detail`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    BadChannel,
    Empty,
    TooLong,
    NoClient,
    NotSent,
    Busy,
    Expired,
    Unsupported,
    Version,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 10] = [
        ErrorCode::BadRequest,
        ErrorCode::BadChannel,
        ErrorCode::Empty,
        ErrorCode::TooLong,
        ErrorCode::NoClient,
        ErrorCode::NotSent,
        ErrorCode::Busy,
        ErrorCode::Expired,
        ErrorCode::Unsupported,
        ErrorCode::Version,
    ];

    /// The wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::BadChannel => "bad_channel",
            ErrorCode::Empty => "empty",
            ErrorCode::TooLong => "too_long",
            ErrorCode::NoClient => "no_client",
            ErrorCode::NotSent => "not_sent",
            ErrorCode::Busy => "busy",
            ErrorCode::Expired => "expired",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Version => "version",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<ErrorCode> {
        ErrorCode::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// A request, app to DLL.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Hello {
        id: u32,
        version: u32,
    },
    /// `channel` stays a string so an unknown channel is `bad_channel`, not `bad_request`.
    Send {
        id: u32,
        channel: String,
        text: String,
    },
    Ping {
        id: u32,
    },
    Bye {
        id: u32,
    },
}

impl Request {
    pub fn id(&self) -> u32 {
        match self {
            Request::Hello { id, .. }
            | Request::Send { id, .. }
            | Request::Ping { id }
            | Request::Bye { id } => *id,
        }
    }

    /// The channel of a `Send`; `BadChannel` if it is not one of [`SendChatChannel::ALL`].
    pub fn send_channel(&self) -> Result<SendChatChannel, ErrorCode> {
        match self {
            Request::Send { channel, .. } => SendChatChannel::ALL
                .into_iter()
                .find(|c| c.as_str() == channel)
                .ok_or(ErrorCode::BadChannel),
            _ => Err(ErrorCode::BadRequest),
        }
    }
}

// Chat text is never logged: Debug prints its length only.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Hello { id, version } => f
                .debug_struct("Hello")
                .field("id", id)
                .field("version", version)
                .finish(),
            Request::Send { id, channel, text } => f
                .debug_struct("Send")
                .field("id", id)
                .field("channel", channel)
                .field("text_len", &text.len())
                .finish(),
            Request::Ping { id } => f.debug_struct("Ping").field("id", id).finish(),
            Request::Bye { id } => f.debug_struct("Bye").field("id", id).finish(),
        }
    }
}

/// A reply, DLL to app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Hello {
        id: u32,
        version: u32,
        dll: String,
    },
    Send {
        id: u32,
        call_id: u32,
    },
    Ack {
        id: u32,
    },
    Error {
        id: u32,
        code: ErrorCode,
        detail: String,
    },
}

/// A line that is not a valid request or reply. `id` is the line's id when it had a
/// valid one, else 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub id: u32,
    pub code: ErrorCode,
}

/// The flat JSON shape of every reply; the fields that do not apply are skipped, so the
/// JSON is exactly the protocol's.
#[derive(Serialize, Deserialize)]
struct WireReply {
    id: u32,
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dll: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    call_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<ErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

fn line_id(value: &serde_json::Value) -> u32 {
    value
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

fn bad(id: u32) -> DecodeError {
    DecodeError {
        id,
        code: ErrorCode::BadRequest,
    }
}

fn to_line<T: Serialize>(value: &T) -> String {
    let mut line = serde_json::to_string(value).expect("plain data serializes");
    line.push('\n');
    line
}

/// Decodes one request line (without its `\n`).
pub fn decode_request(line: &[u8]) -> Result<Request, DecodeError> {
    let value: serde_json::Value = serde_json::from_slice(line).map_err(|_| bad(0))?;
    let id = line_id(&value);
    serde_json::from_value(value).map_err(|_| bad(id))
}

/// The request as one line, `\n` included.
pub fn encode_request(request: &Request) -> String {
    to_line(request)
}

/// Decodes one reply line (without its `\n`).
pub fn decode_reply(line: &[u8]) -> Result<Reply, DecodeError> {
    let wire: WireReply = serde_json::from_slice(line).map_err(|_| {
        let id = serde_json::from_slice::<serde_json::Value>(line)
            .map(|v| line_id(&v))
            .unwrap_or(0);
        bad(id)
    })?;
    let id = wire.id;
    if !wire.ok {
        return match wire.code {
            Some(code) => Ok(Reply::Error {
                id,
                code,
                detail: wire.detail.unwrap_or_default(),
            }),
            None => Err(bad(id)),
        };
    }
    Ok(match (wire.version, wire.dll, wire.call_id) {
        (Some(version), Some(dll), _) => Reply::Hello { id, version, dll },
        (_, _, Some(call_id)) => Reply::Send { id, call_id },
        _ => Reply::Ack { id },
    })
}

/// The reply as one line, `\n` included; an error's `detail` is cut at a char boundary
/// to [`MAX_DETAIL_BYTES`], so a reply always fits [`MAX_LINE_BYTES`].
pub fn encode_reply(reply: &Reply) -> String {
    let mut wire = WireReply {
        id: 0,
        ok: true,
        version: None,
        dll: None,
        call_id: None,
        code: None,
        detail: None,
    };
    match reply {
        Reply::Hello { id, version, dll } => {
            wire.id = *id;
            wire.version = Some(*version);
            wire.dll = Some(dll.clone());
        }
        Reply::Send { id, call_id } => {
            wire.id = *id;
            wire.call_id = Some(*call_id);
        }
        Reply::Ack { id } => wire.id = *id,
        Reply::Error { id, code, detail } => {
            wire.id = *id;
            wire.ok = false;
            wire.code = Some(*code);
            let mut end = detail.len().min(MAX_DETAIL_BYTES);
            while !detail.is_char_boundary(end) {
                end -= 1;
            }
            wire.detail = Some(detail[..end].to_string());
        }
    }
    to_line(&wire)
}

/// A line passed [`MAX_LINE_BYTES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineTooLong;

/// Splits a byte stream into lines under the cap. Once a line is too long the buffer
/// stays failed: the connection is to be closed.
#[derive(Debug, Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
    failed: bool,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `bytes`; returns the lines they completed (without `\n`), in order.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, LineTooLong> {
        if self.failed {
            return Err(LineTooLong);
        }
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        let mut start = 0;
        while let Some(pos) = self.buf[start..].iter().position(|&b| b == b'\n') {
            let end = start + pos;
            if end - start > MAX_LINE_BYTES {
                return self.fail();
            }
            lines.push(self.buf[start..end].to_vec());
            start = end + 1;
        }
        self.buf.drain(..start);
        if self.buf.len() > MAX_LINE_BYTES {
            return self.fail();
        }
        Ok(lines)
    }

    fn fail(&mut self) -> Result<Vec<Vec<u8>>, LineTooLong> {
        self.failed = true;
        self.buf = Vec::new();
        Err(LineTooLong)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SendChatChannel;

    #[test]
    fn constants() {
        assert_eq!(PIPE_PROTOCOL_VERSION, 1);
        assert_eq!(MAX_LINE_BYTES, 4096);
        assert_eq!(MAX_DETAIL_BYTES, 256);
        assert_eq!(WRITE_TIMEOUT_MS, 100);
        assert_eq!(DLL_HOLD_MS, 1000);
        assert_eq!(APP_WAIT_MS, 1500);
        assert_eq!(pipe_name(1234), r"\\.\pipe\resonance-ingame-1234");
    }

    #[test]
    fn error_codes() {
        let names = [
            "bad_request",
            "bad_channel",
            "empty",
            "too_long",
            "no_client",
            "not_sent",
            "busy",
            "expired",
            "unsupported",
            "version",
        ];
        assert_eq!(ErrorCode::ALL.len(), 10);
        for (code, name) in ErrorCode::ALL.iter().zip(names) {
            assert_eq!(code.as_str(), name);
            assert_eq!(serde_json::to_string(code).unwrap(), format!("\"{name}\""));
            assert_eq!(ErrorCode::from_str(name), Some(*code));
        }
        assert_eq!(ErrorCode::from_str("WORLD"), None);
        assert_eq!(ErrorCode::from_str(""), None);
        assert_eq!(ErrorCode::from_str("bad request"), None);
    }

    #[test]
    fn decode_requests() {
        let r = decode_request(br#"{"op":"hello","id":1,"version":1}"#).unwrap();
        assert_eq!(r, Request::Hello { id: 1, version: 1 });
        assert_eq!(r.id(), 1);
        let r = decode_request(br#"{"op":"send","id":2,"channel":"world","text":"hi"}"#).unwrap();
        assert_eq!(
            r,
            Request::Send {
                id: 2,
                channel: "world".into(),
                text: "hi".into()
            }
        );
        assert_eq!(r.id(), 2);
        let r = decode_request(br#"{"op":"ping","id":3}"#).unwrap();
        assert_eq!((r.clone(), r.id()), (Request::Ping { id: 3 }, 3));
        let r = decode_request(br#"{"op":"bye","id":4}"#).unwrap();
        assert_eq!((r.clone(), r.id()), (Request::Bye { id: 4 }, 4));
        let r = decode_request(br#"{"op":"ping","id":9,"extra":[1]}"#).unwrap();
        assert_eq!(r, Request::Ping { id: 9 });
    }

    #[test]
    fn decode_request_errors() {
        let cases: [(&[u8], u32); 8] = [
            (br#"{"op":"nope","id":5}"#, 5),
            (br#"{"op":"send","id":6,"channel":"world"}"#, 6),
            (br#"{"op":"ping","id":"7"}"#, 0),
            (br#"{"op":"ping","id":4294967296}"#, 0),
            (br#"{"op":"ping"}"#, 0),
            (b"abc", 0),
            (b"", 0),
            (&[0xff, 0xfe], 0),
        ];
        for (line, id) in cases {
            assert_eq!(
                decode_request(line),
                Err(DecodeError {
                    id,
                    code: ErrorCode::BadRequest
                }),
                "{:?}",
                String::from_utf8_lossy(line)
            );
        }
    }

    fn send(channel: &str) -> Request {
        Request::Send {
            id: 1,
            channel: channel.into(),
            text: "x".into(),
        }
    }

    #[test]
    fn send_channel() {
        for c in SendChatChannel::ALL {
            assert_eq!(send(c.as_str()).send_channel(), Ok(c));
        }
        for bad in ["whisper", "WORLD", "", " world"] {
            assert_eq!(send(bad).send_channel(), Err(ErrorCode::BadChannel));
        }
        assert_eq!(
            Request::Ping { id: 1 }.send_channel(),
            Err(ErrorCode::BadRequest)
        );
    }

    #[test]
    fn encode_replies() {
        let cases = [
            (
                Reply::Hello {
                    id: 1,
                    version: 1,
                    dll: "0.7.0".into(),
                },
                "{\"id\":1,\"ok\":true,\"version\":1,\"dll\":\"0.7.0\"}\n",
            ),
            (
                Reply::Send { id: 2, call_id: 17 },
                "{\"id\":2,\"ok\":true,\"call_id\":17}\n",
            ),
            (Reply::Ack { id: 3 }, "{\"id\":3,\"ok\":true}\n"),
            (
                Reply::Error {
                    id: 4,
                    code: ErrorCode::NoClient,
                    detail: "udp".into(),
                },
                "{\"id\":4,\"ok\":false,\"code\":\"no_client\",\"detail\":\"udp\"}\n",
            ),
        ];
        for (reply, line) in cases {
            assert_eq!(encode_reply(&reply), line);
            assert_eq!(
                decode_reply(line.trim_end_matches('\n').as_bytes()),
                Ok(reply)
            );
        }
    }

    #[test]
    fn encode_requests() {
        let hello = Request::Hello { id: 1, version: 1 };
        let line = encode_request(&hello);
        assert_eq!(line, "{\"op\":\"hello\",\"id\":1,\"version\":1}\n");
        assert_eq!(decode_request(line.trim_end().as_bytes()), Ok(hello));
        let s = Request::Send {
            id: 2,
            channel: "say".into(),
            text: "あ😀".into(),
        };
        let line = encode_request(&s);
        assert!(line.ends_with('\n'));
        assert_eq!(decode_request(line.trim_end().as_bytes()), Ok(s));
        for r in [Request::Ping { id: 3 }, Request::Bye { id: 4 }] {
            let line = encode_request(&r);
            assert_eq!(decode_request(line.trim_end().as_bytes()), Ok(r));
        }
    }

    fn detail_of(line: &str) -> String {
        match decode_reply(line.trim_end().as_bytes()).unwrap() {
            Reply::Error { detail, .. } => detail,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn detail_is_cut() {
        let err = |detail: String| Reply::Error {
            id: 1,
            code: ErrorCode::Unsupported,
            detail,
        };
        let line = encode_reply(&err("a".repeat(5000)));
        assert!(line.len() <= MAX_LINE_BYTES);
        assert_eq!(detail_of(&line).len(), MAX_DETAIL_BYTES);
        let line = encode_reply(&err("가".repeat(200)));
        let d = detail_of(&line);
        assert_eq!(d.chars().count(), 85);
        assert_eq!(d.len(), 255);
    }

    #[test]
    fn escaped_text_fits_the_cap() {
        let mut text = String::new();
        let mut json = String::new();
        for _ in 0..140 {
            text.push('あ');
            json.push_str("\\u3042");
        }
        let line = format!(r#"{{"op":"send","id":1,"channel":"world","text":"{json}"}}"#);
        assert!(line.len() < MAX_LINE_BYTES);
        match decode_request(line.as_bytes()).unwrap() {
            Request::Send { text: t, .. } => assert_eq!(t, text),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn debug_hides_the_text() {
        let s = format!(
            "{:?}",
            Request::Send {
                id: 1,
                channel: "world".into(),
                text: "secret".into()
            }
        );
        assert!(s.contains("text_len") && s.contains('6'));
        assert!(!s.contains("secret"));
    }

    #[test]
    fn line_buffer() {
        let mut b = LineBuffer::new();
        assert_eq!(b.push(b"ping\n").unwrap(), vec![b"ping".to_vec()]);
        assert!(b.push(b"pi").unwrap().is_empty());
        assert_eq!(b.push(b"ng\n").unwrap(), vec![b"ping".to_vec()]);
        assert_eq!(
            b.push(b"a\nb\nc").unwrap(),
            vec![b"a".to_vec(), b"b".to_vec()]
        );
        assert_eq!(b.push(b"\n").unwrap(), vec![b"c".to_vec()]);
        assert_eq!(b.push(b"\n").unwrap(), vec![Vec::<u8>::new()]);
    }

    #[test]
    fn line_buffer_cap() {
        let mut b = LineBuffer::new();
        let mut ok = vec![b'a'; MAX_LINE_BYTES];
        ok.push(b'\n');
        assert_eq!(b.push(&ok).unwrap().len(), 1);

        let mut b = LineBuffer::new();
        let mut long = vec![b'a'; MAX_LINE_BYTES + 1];
        long.push(b'\n');
        assert_eq!(b.push(&long), Err(LineTooLong));
        assert_eq!(b.push(b"ping\n"), Err(LineTooLong));

        let mut b = LineBuffer::new();
        assert_eq!(b.push(&[b'a'; 5000]), Err(LineTooLong));
        assert_eq!(b.push(b"\n"), Err(LineTooLong));

        // The cap holds across pushes too.
        let mut b = LineBuffer::new();
        assert!(b.push(&vec![b'a'; 4000]).unwrap().is_empty());
        assert_eq!(b.push(&[b'a'; 200]), Err(LineTooLong));
    }
}
