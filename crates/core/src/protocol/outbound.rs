//! Reads the outbound frames of `resonance-frame` back with this crate's own
//! parser. A frame built from the documented layout is well-formed protobuf
//! with the documented field numbers; this does not show the server accepts it.

use crate::protocol::decoder::{find_int_by_tag, find_string_by_tag, Fields, Value};
use crate::protocol::framing::{FrameAssembler, PREFIX_LEN};
use resonance_frame::{build_send_chat, WireChannel, HEADER_LEN};
use resonance_types::{prepare_text, SendChatChannel};

fn fields(data: &[u8]) -> Vec<(u64, Value<'_>)> {
    Fields::new(data).map(|f| (f.number(), f.value)).collect()
}

fn bytes_of<'a>(value: Value<'a>) -> &'a [u8] {
    match value {
        Value::Bytes(b) => b,
        other => panic!("expected bytes, got {other:?}"),
    }
}

fn inputs() -> Vec<(WireChannel, String)> {
    vec![
        (WireChannel::World, "hi".to_string()),
        (WireChannel::Say, "こんにちは".to_string()),
        (WireChannel::Guild, "a".repeat(140)),
        (WireChannel::Party, "あ".repeat(140)),
    ]
}

#[test]
fn prefix_agrees_with_inbound_framing() {
    let frame = build_send_chat(WireChannel::World, "hi", 1).unwrap();
    assert_eq!(
        u32::from_be_bytes(frame[0..4].try_into().unwrap()) as usize,
        frame.len()
    );
    assert_eq!(u16::from_be_bytes(frame[4..6].try_into().unwrap()), 1);
    assert_eq!(PREFIX_LEN, resonance_frame::PREFIX_LEN);
}

#[test]
fn payload_reads_back_as_protobuf() {
    for (channel, text) in inputs() {
        let frame = build_send_chat(channel, &text, 1).unwrap();
        let payload = &frame[HEADER_LEN..];

        let outer = fields(payload);
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].0, 1);
        let request = bytes_of(outer[0].1);
        // the field 1 body ends exactly at the end of the frame
        assert!(std::ptr::eq(
            request.as_ptr_range().end,
            payload.as_ptr_range().end
        ));

        let req = fields(request);
        assert_eq!(req.len(), 2);
        assert_eq!(req[0], (2, Value::Varint(u64::from(channel.value()))));
        assert_eq!(req[1].0, 4);
        let info = bytes_of(req[1].1);

        let inf = fields(info);
        assert_eq!(inf.len(), 2);
        assert_eq!(inf[0], (1, Value::Varint(0)));
        assert_eq!(inf[1], (3, Value::Bytes(text.as_bytes())));

        assert_eq!(
            find_int_by_tag(request, 0x10),
            Some(u64::from(channel.value()))
        );
        assert_eq!(find_string_by_tag(info, 0x1A), Some(text));
    }
}

#[test]
fn outbound_frames_are_not_taken_for_inbound() {
    let g1 = build_send_chat(WireChannel::World, "hi", 1).unwrap();
    let g2 = build_send_chat(WireChannel::Say, "こんにちは", 0x8000_0001).unwrap();
    assert!(FrameAssembler::new().push(&g1).is_empty());
    assert!(FrameAssembler::new().push(&g2).is_empty());
}

#[test]
fn channel_names_map_to_wire_values() {
    assert_eq!(SendChatChannel::ALL.len(), 4);
    let values: Vec<u32> = SendChatChannel::ALL
        .iter()
        .map(|c| WireChannel::from_name(c.as_str()).unwrap().value())
        .collect();
    assert_eq!(values, [1, 4, 3, 2]);
}

#[test]
fn text_limit_agrees_with_prepare_text() {
    assert_eq!(
        resonance_frame::MAX_TEXT_UTF16,
        resonance_types::MAX_TEXT_UTF16
    );
    for text in [
        "a".repeat(140),
        "a".repeat(141),
        "😀".repeat(70),
        "😀".repeat(71),
    ] {
        let built = build_send_chat(WireChannel::Say, &text, 1).is_ok();
        assert_eq!(
            built,
            prepare_text(&text).is_ok(),
            "{} units",
            text.encode_utf16().count()
        );
    }
}
