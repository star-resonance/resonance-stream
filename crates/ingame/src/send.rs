#[cfg(test)]
mod tests {
    use super::*;
    use resonance_frame::WireChannel;
    use resonance_types::ingame_pipe::{encode_reply, ErrorCode, Reply};
    use resonance_types::SendChatChannel;

    fn detail_of(r: Result<&str, TextError>) -> (ErrorCode, String) {
        let e = r.unwrap_err();
        (e.code(), e.detail())
    }

    #[test]
    fn recheck_trims_and_accepts() {
        assert_eq!(recheck_text("  hi  "), Ok("hi"));
        assert_eq!(recheck_text(&"a".repeat(140)), Ok(&*"a".repeat(140)));
        assert!(recheck_text(&"😀".repeat(70)).is_ok());
        assert!(recheck_text(&"あ".repeat(140)).is_ok());
        let padded = format!(" {} ", "a".repeat(140));
        assert_eq!(recheck_text(&padded).map(str::len), Ok(140));
    }

    #[test]
    fn recheck_empty() {
        for t in ["", "   ", "\t", "\u{3000}", "\n", " \n "] {
            assert_eq!(recheck_text(t), Err(TextError::Empty), "{t:?}");
        }
        let e = TextError::Empty;
        assert_eq!(e.code(), ErrorCode::Empty);
    }

    #[test]
    fn recheck_newlines_are_too_long() {
        for t in ["a\nb", "a\rb"] {
            assert_eq!(
                detail_of(recheck_text(t)),
                (ErrorCode::TooLong, "newline".to_string())
            );
        }
    }

    #[test]
    fn recheck_other_control_characters_are_bad_request() {
        for (t, detail) in [
            ("a\tb", "control_char:U+0009"),
            ("a\u{0}b", "control_char:U+0000"),
            ("a\u{7f}b", "control_char:U+007F"),
            ("a\u{85}b", "control_char:U+0085"),
            ("a\u{2028}b", "control_char:U+2028"),
            ("a\u{2029}b", "control_char:U+2029"),
        ] {
            assert_eq!(
                detail_of(recheck_text(t)),
                (ErrorCode::BadRequest, detail.to_string())
            );
        }
    }

    #[test]
    fn recheck_length_counts_utf16_units() {
        assert_eq!(
            detail_of(recheck_text(&"a".repeat(141))),
            (ErrorCode::TooLong, "units:141".to_string())
        );
        assert_eq!(
            detail_of(recheck_text(&"😀".repeat(71))),
            (ErrorCode::TooLong, "units:142".to_string())
        );
    }

    #[test]
    fn details_never_hold_the_text() {
        for t in ["a\nsecret", "secret\tx", "secret\u{0}"] {
            let (_, d) = detail_of(recheck_text(t));
            assert!(!d.contains("secret") && !d.contains('a'), "{d}");
        }
    }

    #[test]
    fn call_ids_stay_in_the_upper_half() {
        assert_eq!(CallIds::from_seed(0).next(), 0x8000_0000);
        assert_eq!(CallIds::from_seed(0x1234_5678).next(), 0x9234_5678);
        let mut ids = CallIds::from_seed(1);
        assert_eq!((ids.next(), ids.next()), (0x8000_0001, 0x8000_0002));
        let mut ids = CallIds::from_seed(u32::MAX);
        assert_eq!(
            (ids.next(), ids.next(), ids.next()),
            (0xFFFF_FFFF, 0x8000_0000, 0x8000_0001)
        );
        let mut ids = CallIds::new();
        let values: Vec<u32> = (0..100).map(|_| ids.next()).collect();
        assert!(values.iter().all(|v| *v >= 0x8000_0000));
        assert!(values.iter().any(|v| *v != values[0]));
    }

    fn line(r: Reply) -> String {
        encode_reply(&r)
    }

    #[test]
    fn replies_map_outcomes() {
        assert_eq!(
            line(reply_for(2, 17, SendOutcome::Sent)),
            "{\"id\":2,\"ok\":true,\"call_id\":17}\n"
        );
        let cases = [
            (
                SendOutcome::NoClient("udp".into()),
                ErrorCode::NoClient,
                "udp",
            ),
            (
                SendOutcome::Unsupported("method_missing:Send".into()),
                ErrorCode::Unsupported,
                "method_missing:Send",
            ),
            (
                SendOutcome::NotSent("exception:Send".into()),
                ErrorCode::NotSent,
                "exception:Send",
            ),
            (SendOutcome::Busy, ErrorCode::Busy, "send already waiting"),
            (
                SendOutcome::Expired,
                ErrorCode::Expired,
                "main thread did not tick in 1000 ms",
            ),
            (
                SendOutcome::Disabled,
                ErrorCode::Unsupported,
                "pump disabled",
            ),
            (
                SendOutcome::HookFailed("MovingCodeNotSupported".into()),
                ErrorCode::Unsupported,
                "hook_failed:MovingCodeNotSupported",
            ),
        ];
        for (outcome, code, detail) in cases {
            assert_eq!(
                reply_for(5, 99, outcome),
                Reply::Error {
                    id: 5,
                    code,
                    detail: detail.into()
                }
            );
        }
    }

    #[derive(Default)]
    struct FakeQueue {
        frames: Vec<Vec<u8>>,
        answer: Option<SendOutcome>,
    }

    impl SendQueue for FakeQueue {
        fn submit(&mut self, frame: &[u8]) -> SendOutcome {
            self.frames.push(frame.to_vec());
            self.answer.clone().unwrap_or(SendOutcome::Sent)
        }
    }

    const G2: [u8; 51] = [
        0x00, 0x00, 0x00, 0x33, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x4E, 0xBF, 0xDF, 0x38, 0x00,
        0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0A, 0x17, 0x10, 0x02,
        0x22, 0x13, 0x08, 0x00, 0x1A, 0x0F, 0xE3, 0x81, 0x93, 0xE3, 0x82, 0x93, 0xE3, 0x81, 0xAB,
        0xE3, 0x81, 0xA1, 0xE3, 0x81, 0xAF,
    ];

    fn service() -> SendService<FakeQueue> {
        SendService::new(FakeQueue::default(), CallIds::from_seed(1))
    }

    #[test]
    fn service_sends_the_golden_frame() {
        let mut s = service();
        assert_eq!(
            s.send(2, "say", "こんにちは"),
            Reply::Send {
                id: 2,
                call_id: 0x8000_0001
            }
        );
        assert_eq!(s.queue().frames, vec![G2.to_vec()]);
        assert_eq!(
            s.send(3, "say", "こんにちは"),
            Reply::Send {
                id: 3,
                call_id: 0x8000_0002
            }
        );
    }

    #[test]
    fn service_sends_the_trimmed_text() {
        let mut s = service();
        s.send(2, "say", "  こんにちは \n");
        assert_eq!(s.queue().frames, vec![G2.to_vec()]);
    }

    #[test]
    fn rejections_touch_neither_queue_nor_ids() {
        let mut s = service();
        for (channel, text, code) in [
            ("whisper", "hi", ErrorCode::BadChannel),
            ("say", "", ErrorCode::Empty),
            ("say", "a\tb", ErrorCode::BadRequest),
            ("say", "a\nb", ErrorCode::TooLong),
        ] {
            match s.send(2, channel, text) {
                Reply::Error { id: 2, code: c, .. } => assert_eq!(c, code),
                other => panic!("{other:?}"),
            }
        }
        assert!(s.queue().frames.is_empty());
        assert_eq!(
            s.send(2, "world", "hi"),
            Reply::Send {
                id: 2,
                call_id: 0x8000_0001
            }
        );
    }

    #[test]
    fn every_pipe_channel_is_a_wire_channel() {
        for c in SendChatChannel::ALL {
            assert!(WireChannel::from_name(c.as_str()).is_some(), "{c:?}");
        }
    }

    #[test]
    fn a_busy_queue_is_busy() {
        let mut s = SendService::new(
            FakeQueue {
                answer: Some(SendOutcome::Busy),
                ..Default::default()
            },
            CallIds::from_seed(1),
        );
        match s.send(2, "world", "hi") {
            Reply::Error { code, .. } => assert_eq!(code, ErrorCode::Busy),
            other => panic!("{other:?}"),
        }
    }
}
