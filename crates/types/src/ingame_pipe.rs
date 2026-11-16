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
        assert_eq!(b.push(&vec![b'a'; 5000]), Err(LineTooLong));
        assert_eq!(b.push(b"\n"), Err(LineTooLong));

        // The cap holds across pushes too.
        let mut b = LineBuffer::new();
        assert!(b.push(&vec![b'a'; 4000]).unwrap().is_empty());
        assert_eq!(b.push(&vec![b'a'; 200]), Err(LineTooLong));
    }
}
