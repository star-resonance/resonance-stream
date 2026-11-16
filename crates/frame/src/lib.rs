//! Builds the outbound `SendChitChatMsg` Call frame of the game's port-5003
//! chat protocol: a 6-byte prefix (`size`, `flags`), a 20-byte inner header
//! (service uuid, stub id, call id, method id) and a protobuf body. All header
//! integers are big-endian and `size` counts the whole frame.
//!
//! `no_std`, no dependencies, and no `unwrap`, `expect` or computed indexing
//! outside the tests: the caller runs in the game's process. (It allocates the
//! returned `Vec`, so an allocation failure still aborts.)
//!
//! **Derived, not observed.** The layout comes from `docs/ingame-design.md`.
//! The prefix agrees with core's inbound framing; the rest of the header and
//! the protobuf field numbers are the document's, and the corpus holds no
//! outbound frame. The goldens prove the builder matches that layout, not that
//! the server accepts it.
//!
//! **NOT VERIFIED** (only a run in the game settles it): the server accepts a
//! `service_uuid` whose high 32 bits are zero; a caller-chosen `call_id` can
//! equal one of the game's, so avoiding that is the caller's job; and the
//! explicit `msg_type` field (`08 00`) is what the game's own client writes.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

/// Bytes of the whole header: prefix plus inner header.
pub const HEADER_LEN: usize = 26;
/// Bytes of the prefix: `[size u32][flags u16]`.
pub const PREFIX_LEN: usize = 6;
/// `flags` of a Call (bit 15 would mean zstd; never set here).
pub const FLAGS_CALL: u16 = 1;
/// The ChitChat service uuid (`0x4EBFDF38`), high 32 bits zero.
pub const CHITCHAT_SERVICE: u64 = 1_321_197_368;
/// The `SendChitChatMsg` method id.
pub const METHOD_SEND_CHITCHAT_MSG: u32 = 1;
/// The game's limit on a message, in UTF-16 code units.
pub const MAX_TEXT_UTF16: usize = 140;

/// A channel the game takes a message on (whispers are not supported).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WireChannel {
    World,
    Say,
    Party,
    Guild,
}

impl WireChannel {
    /// The `channel_type` written on the wire.
    pub fn value(self) -> u32 {
        match self {
            WireChannel::World => 1,
            WireChannel::Say => 2,
            WireChannel::Party => 3,
            WireChannel::Guild => 4,
        }
    }

    /// The lower-case name the pipe carries.
    pub fn name(self) -> &'static str {
        match self {
            WireChannel::World => "world",
            WireChannel::Say => "say",
            WireChannel::Party => "party",
            WireChannel::Guild => "guild",
        }
    }

    /// Exactly the four lower-case names of [`WireChannel::name`].
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "world" => Some(WireChannel::World),
            "say" => Some(WireChannel::Say),
            "party" => Some(WireChannel::Party),
            "guild" => Some(WireChannel::Guild),
            _ => None,
        }
    }
}

/// Why a frame was not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The text is longer than [`MAX_TEXT_UTF16`]; `units` is the count found.
    TextTooLong { units: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TextTooLong { units } => write!(
                f,
                "text is {units} UTF-16 units, the limit is {MAX_TEXT_UTF16}"
            ),
        }
    }
}

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Builds the Call frame for `text` on `channel` with the caller's `call_id`.
///
/// Checks the 140-unit limit and nothing else: it does not trim, does not
/// replace CR/LF and accepts empty text (those are the callers' rules). The
/// `call_id` is written as given. See the crate docs for what is NOT VERIFIED.
pub fn build_send_chat(
    channel: WireChannel,
    text: &str,
    call_id: u32,
) -> Result<Vec<u8>, FrameError> {
    let units: usize = text.chars().map(char::len_utf16).sum();
    if units > MAX_TEXT_UTF16 {
        return Err(FrameError::TextTooLong { units });
    }
    let bytes = text.as_bytes();

    // ChatMsgInfo { 1: msg_type 0, 3: msg_text }
    let mut info = Vec::with_capacity(bytes.len() + 5);
    info.extend_from_slice(&[0x08, 0x00, 0x1A]);
    push_varint(&mut info, bytes.len() as u64);
    info.extend_from_slice(bytes);

    // SendChitChatMsgRequest { 2: channel_type, 4: ChatMsgInfo }
    let mut request = Vec::with_capacity(info.len() + 6);
    request.push(0x10);
    push_varint(&mut request, u64::from(channel.value()));
    request.push(0x22);
    push_varint(&mut request, info.len() as u64);
    request.extend_from_slice(&info);

    // SendChitChatMsg { 1: request }
    let mut payload = Vec::with_capacity(request.len() + 3);
    payload.push(0x0A);
    push_varint(&mut payload, request.len() as u64);
    payload.extend_from_slice(&request);

    // At most 459 bytes in all, so the size always fits a u32.
    let size = (HEADER_LEN + payload.len()) as u32;
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&size.to_be_bytes());
    frame.extend_from_slice(&FLAGS_CALL.to_be_bytes());
    frame.extend_from_slice(&CHITCHAT_SERVICE.to_be_bytes());
    frame.extend_from_slice(&0u32.to_be_bytes());
    frame.extend_from_slice(&call_id.to_be_bytes());
    frame.extend_from_slice(&METHOD_SEND_CHITCHAT_MSG.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;

    const G1: &[u8] = &[
        0x00, 0x00, 0x00, 0x26, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x4E, 0xBF, 0xDF, 0x38, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0A, 0x0A, 0x10, 0x01,
        0x22, 0x06, 0x08, 0x00, 0x1A, 0x02, 0x68, 0x69,
    ];

    const G2: &[u8] = &[
        0x00, 0x00, 0x00, 0x33, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x4E, 0xBF, 0xDF, 0x38, 0x00,
        0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0A, 0x17, 0x10, 0x02,
        0x22, 0x13, 0x08, 0x00, 0x1A, 0x0F, 0xE3, 0x81, 0x93, 0xE3, 0x82, 0x93, 0xE3, 0x81, 0xAB,
        0xE3, 0x81, 0xA1, 0xE3, 0x81, 0xAF,
    ];

    const G3_HEAD: &[u8] = &[
        0x00, 0x00, 0x00, 0xB3, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x4E, 0xBF, 0xDF, 0x38, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x01, 0x0A, 0x96, 0x01, 0x10,
        0x04, 0x22, 0x91, 0x01, 0x08, 0x00, 0x1A, 0x8C, 0x01,
    ];

    const G4_HEAD: &[u8] = &[
        0x00, 0x00, 0x01, 0xCB, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x4E, 0xBF, 0xDF, 0x38, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0A, 0xAE, 0x03, 0x10,
        0x03, 0x22, 0xA9, 0x03, 0x08, 0x00, 0x1A, 0xA4, 0x03,
    ];

    fn g3() -> Vec<u8> {
        let mut v = G3_HEAD.to_vec();
        v.extend(core::iter::repeat(0x61u8).take(140));
        v
    }

    fn g4() -> Vec<u8> {
        let mut v = G4_HEAD.to_vec();
        for _ in 0..140 {
            v.extend_from_slice(&[0xE3, 0x81, 0x82]);
        }
        v
    }

    fn repeat(s: &str, n: usize) -> String {
        s.repeat(n)
    }

    #[test]
    fn goldens() {
        assert_eq!(
            build_send_chat(WireChannel::World, "hi", 1),
            Ok(G1.to_vec())
        );
        assert_eq!(
            build_send_chat(WireChannel::Say, "こんにちは", 0x8000_0001),
            Ok(G2.to_vec())
        );
        let g3 = g3();
        assert_eq!(g3.len(), 179);
        assert_eq!(
            build_send_chat(WireChannel::Guild, &repeat("a", 140), 5),
            Ok(g3)
        );
        let g4 = g4();
        assert_eq!(g4.len(), 459);
        assert_eq!(
            build_send_chat(WireChannel::Party, &repeat("あ", 140), 1),
            Ok(g4)
        );
    }

    #[test]
    fn header_fields_by_offset() {
        let frames: [Vec<u8>; 4] = [G1.to_vec(), G2.to_vec(), g3(), g4()];
        for f in &frames {
            assert_eq!(
                u32::from_be_bytes([f[0], f[1], f[2], f[3]]) as usize,
                f.len()
            );
            assert_eq!(&f[4..6], &[0, 1]);
        }
        let f = build_send_chat(WireChannel::World, "hi", 7).unwrap();
        assert_eq!(&f[6..14], &[0, 0, 0, 0, 0x4E, 0xBF, 0xDF, 0x38]);
        let mut uuid = [0u8; 8];
        uuid.copy_from_slice(&f[6..14]);
        assert_eq!(u64::from_be_bytes(uuid), 1_321_197_368);
        assert_eq!(&f[14..18], &[0, 0, 0, 0]);
        assert_eq!(&f[18..22], &[0, 0, 0, 7]);
        assert_eq!(&f[22..26], &[0, 0, 0, 1]);
        assert_eq!(HEADER_LEN, 26);
        assert_eq!(PREFIX_LEN, 6);
        assert_eq!(FLAGS_CALL, 1);
        assert_eq!(CHITCHAT_SERVICE, 1_321_197_368);
        assert_eq!(METHOD_SEND_CHITCHAT_MSG, 1);
        assert_eq!(MAX_TEXT_UTF16, 140);
    }

    #[test]
    fn channel_byte_and_names() {
        let all = [
            (WireChannel::World, 1u32, "world"),
            (WireChannel::Say, 2, "say"),
            (WireChannel::Party, 3, "party"),
            (WireChannel::Guild, 4, "guild"),
        ];
        let base = build_send_chat(WireChannel::World, "hi", 1).unwrap();
        for (c, v, n) in all {
            assert_eq!(c.value(), v);
            assert_eq!(c.name(), n);
            assert_eq!(WireChannel::from_name(n), Some(c));
            let f = build_send_chat(c, "hi", 1).unwrap();
            assert_eq!(f[29] as u32, v);
            assert_eq!(f.len(), base.len());
            for i in 0..f.len() {
                if i != 29 {
                    assert_eq!(f[i], base[i]);
                }
            }
        }
        for bad in ["WORLD", "whisper", "", "world "] {
            assert_eq!(WireChannel::from_name(bad), None);
        }
    }

    #[test]
    fn call_id_is_verbatim() {
        for (id, bytes) in [
            (0u32, [0u8, 0, 0, 0]),
            (1, [0, 0, 0, 1]),
            (0xFFFF_FFFF, [0xFF; 4]),
        ] {
            let f = build_send_chat(WireChannel::Say, "x", id).unwrap();
            assert_eq!(&f[18..22], &bytes);
        }
    }

    #[test]
    fn text_limit() {
        use core::fmt::Write;
        assert!(build_send_chat(WireChannel::Say, &repeat("a", 140), 1).is_ok());
        assert_eq!(
            build_send_chat(WireChannel::Say, &repeat("a", 141), 1),
            Err(FrameError::TextTooLong { units: 141 })
        );
        assert!(build_send_chat(WireChannel::Say, &repeat("😀", 70), 1).is_ok());
        assert_eq!(
            build_send_chat(WireChannel::Say, &repeat("😀", 71), 1),
            Err(FrameError::TextTooLong { units: 142 })
        );
        assert!(build_send_chat(WireChannel::Say, &repeat("あ", 140), 1).is_ok());
        let mut msg = String::new();
        write!(msg, "{}", FrameError::TextTooLong { units: 141 }).unwrap();
        assert!(msg.contains("141") && msg.contains("140") && !msg.contains('\n'));
    }

    #[test]
    fn text_is_framed_untouched() {
        let f = build_send_chat(WireChannel::Say, "a\nb", 1).unwrap();
        assert_eq!(&f[f.len() - 3..], &[0x61, 0x0A, 0x62]);
        assert_eq!(
            u32::from_be_bytes([f[0], f[1], f[2], f[3]]) as usize,
            f.len()
        );
    }
}
