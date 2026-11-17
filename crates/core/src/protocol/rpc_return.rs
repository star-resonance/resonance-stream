//! The server's Return to a `SendChitChatMsg` call, read from a plain `0x0003` frame.
//!
//! Layout (big-endian; from the Stellar wire tap, NOT VERIFIED against a capture): the frame's
//! 12-byte inner header is `stub_id` (bytes 0..4), `call_id` (4..8), `error_id` (8..12); the root
//! is `0A <len> <SendChitChatMsgReply>` with field 3 `show_msg` (its own field 3 is the chat
//! message's Unix-second timestamp), field 4 `cd_end_time` (varint, Unix seconds) and field 5
//! `err_code` (varint).
//!
//! A Return carries no service or method, so this reads the Return of every RPC; a decoded code
//! means something only for a `call_id` the caller knows it sent.

use crate::protocol::decoder::{field_end, read_varint, Fields, Value};

const ROOT_TAG: u8 = 0x0A;

/// `err_code` of an accepted send.
pub const ERR_ACCEPTED: u32 = 0;
/// `err_code`: the chat-send cooldown has not ended (`cd_end_time` says when).
pub const ERR_CHAT_SEND_CD_NO_END: u32 = 5211;

/// A decoded Return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcReturn {
    pub call_id: u32,
    pub err_code: u32,
    /// Unix seconds; 0 = none.
    pub cd_end_time: u64,
    /// The inner header's bytes 8..12.
    pub error_id: u32,
    /// The chat message's Unix-second timestamp from `show_msg`, when present and nonzero.
    pub server_time: Option<u64>,
}

/// What a Return says about a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Accepted,
    CooldownNotEnded,
    /// Any other code: kept, never named.
    Other(u32),
}

impl RpcReturn {
    pub fn outcome(&self) -> SendOutcome {
        match self.err_code {
            ERR_ACCEPTED => SendOutcome::Accepted,
            ERR_CHAT_SEND_CD_NO_END => SendOutcome::CooldownNotEnded,
            n => SendOutcome::Other(n),
        }
    }
}

/// Reads a Return from a frame's inner header and root. `None` when the header is shorter than
/// 12 bytes, the root is not `0A <len> ..`, or `err_code` does not fit a `u32`.
pub fn decode(inner_header: &[u8], root: &[u8]) -> Option<RpcReturn> {
    let header: &[u8; 12] = inner_header.get(..12)?.try_into().ok()?;
    let word = |at: usize| {
        u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    if root.first() != Some(&ROOT_TAG) {
        return None;
    }
    let (len, read) = read_varint(&root[1..]);
    let start = 1 + read;
    let end = field_end(start, len, root.len());
    let (mut err_code, mut cd_end_time, mut server_time) = (0u64, 0u64, None);
    for field in Fields::new(&root[start..end]) {
        match (field.number(), field.value) {
            (3, Value::Bytes(show)) => server_time = show_msg_time(show),
            (4, Value::Varint(n)) => cd_end_time = n,
            (5, Value::Varint(n)) => err_code = n,
            _ => {}
        }
    }
    Some(RpcReturn {
        call_id: word(4),
        err_code: u32::try_from(err_code).ok()?,
        cd_end_time,
        error_id: word(8),
        server_time,
    })
}

/// The chat message's timestamp: field 3 of `show_msg`, when a nonzero varint.
fn show_msg_time(show: &[u8]) -> Option<u64> {
    Fields::new(show)
        .filter_map(|f| match (f.number(), f.value) {
            (3, Value::Varint(n)) => Some(n),
            _ => None,
        })
        .last()
        .filter(|&n| n != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_790_000_005;

    fn h(stub: u32, call: u32, err: u32) -> Vec<u8> {
        [stub, call, err]
            .iter()
            .flat_map(|n| n.to_be_bytes())
            .collect()
    }

    fn varint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (n & 0x7F) as u8;
            n >>= 7;
            if n == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    /// A varint field.
    fn num(field: u64, value: u64) -> Vec<u8> {
        [varint(field << 3), varint(value)].concat()
    }

    /// A bytes field.
    fn bytes(field: u64, body: &[u8]) -> Vec<u8> {
        [
            varint(field << 3 | 2),
            varint(body.len() as u64),
            body.to_vec(),
        ]
        .concat()
    }

    /// `0A <len> <fields>`.
    fn root(fields: &[Vec<u8>]) -> Vec<u8> {
        let body = fields.concat();
        [vec![0x0A], varint(body.len() as u64), body].concat()
    }

    fn ret(call_id: u32, err_code: u32, cd_end_time: u64) -> RpcReturn {
        RpcReturn {
            call_id,
            err_code,
            cd_end_time,
            error_id: 0,
            server_time: None,
        }
    }

    #[test]
    fn an_accepted_send() {
        let chat = bytes(3, &num(3, T));
        let got = decode(&h(0, 0x8000_0011, 0), &root(&[chat, num(4, T)])).unwrap();
        assert_eq!(
            got,
            RpcReturn {
                server_time: Some(T),
                ..ret(0x8000_0011, 0, T)
            }
        );
        assert_eq!(got.outcome(), SendOutcome::Accepted);
    }

    #[test]
    fn a_refused_send_names_its_cooldown() {
        assert_eq!(ERR_CHAT_SEND_CD_NO_END, 5211);
        for fields in [[num(4, T), num(5, 5211)], [num(5, 5211), num(4, T)]] {
            let got = decode(&h(0, 7, 0), &root(&fields)).unwrap();
            assert_eq!(got, ret(7, 5211, T));
            assert_eq!(got.outcome(), SendOutcome::CooldownNotEnded);
        }
    }

    #[test]
    fn a_refusal_without_a_cooldown_end() {
        let got = decode(&h(0, 7, 0), &root(&[num(5, 5211)])).unwrap();
        assert_eq!(got.cd_end_time, 0);
    }

    #[test]
    fn other_codes_are_kept() {
        let got = decode(&h(0, 7, 0), &root(&[num(5, 9999)])).unwrap();
        assert_eq!(
            (got.err_code, got.outcome()),
            (9999, SendOutcome::Other(9999))
        );
        let got = decode(&h(0, 7, 0), &root(&[num(5, 1)])).unwrap();
        assert_eq!(got.outcome(), SendOutcome::Other(1));
    }

    #[test]
    fn an_empty_reply_is_accepted() {
        let got = decode(&h(0, 7, 0), &[0x0A, 0x00]).unwrap();
        assert_eq!(got, ret(7, 0, 0));
        assert_eq!(got.outcome(), SendOutcome::Accepted);
    }

    #[test]
    fn call_id_is_bytes_4_to_8_big_endian() {
        let got = decode(&h(0x1111_1111, 0x2222_2222, 0x3333_3333), &[0x0A, 0x00]).unwrap();
        assert_eq!(got.call_id, 0x2222_2222);
        let mut header = [0u8; 12];
        header[4..8].copy_from_slice(&[0x80, 0, 0, 0x11]);
        assert_eq!(decode(&header, &[0x0A, 0x00]).unwrap().call_id, 0x8000_0011);
    }

    #[test]
    fn unknown_fields_and_repeats_are_tolerated() {
        let fields = [
            num(9, 77),
            bytes(7, b"abc"),
            num(300, 1), // a two-byte tag
            num(5, 5211),
            num(5, 9),
            num(9, 1),
        ];
        let got = decode(&h(0, 7, 0), &root(&fields)).unwrap();
        assert_eq!(got.err_code, 9);
        // Field 4 as bytes (the wrong wire type) is ignored.
        let got = decode(&h(0, 7, 0), &root(&[bytes(4, &[1, 2]), num(5, 5211)])).unwrap();
        assert_eq!((got.cd_end_time, got.err_code), (0, 5211));
    }

    #[test]
    fn what_is_not_a_return_is_none() {
        let good = root(&[num(5, 5211)]);
        for n in 0..12 {
            assert_eq!(decode(&h(0, 7, 0)[..n], &good), None, "header of {n}");
        }
        assert_eq!(decode(&h(0, 7, 0), &[]), None);
        assert_eq!(decode(&h(0, 7, 0), &[0x12, 0x00]), None);
        let wide = root(&[num(5, u64::from(u32::MAX) + 1)]);
        assert_eq!(decode(&h(0, 7, 0), &wide), None);
        let minus_one = root(&[num(5, u64::MAX)]); // int32 -1 is a 10-byte varint
        assert_eq!(decode(&h(0, 7, 0), &minus_one), None);
        assert!(decode(&h(0, 7, 0), &root(&[num(5, u64::from(u32::MAX))])).is_some());
    }

    #[test]
    fn corrupt_input_never_panics() {
        let header = h(1, 2, 3);
        // A length running past the end, an 11-byte varint.
        let _ = decode(&header, &[0x0A, 0x7F, 0x28, 0x01]);
        let _ = decode(
            &header,
            &[
                0x0A, 0x0C, 0x28, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
            ],
        );
        // Every truncation of a valid root.
        let full = root(&[bytes(3, &num(3, T)), num(4, T), num(5, 5211)]);
        for cut in 0..=full.len() {
            let _ = decode(&header, &full[..cut]);
        }
        // Random short roots.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = (next() % 65) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let _ = decode(&header, &bytes);
        }
    }

    #[test]
    fn send_outcome_is_plain_data() {
        fn copy<T: Copy>(_: T) {}
        copy(SendOutcome::Other(3));
        assert_eq!(SendOutcome::Other(3), SendOutcome::Other(3));
        assert_ne!(SendOutcome::Other(3), SendOutcome::Other(4));
    }

    #[test]
    fn error_id_is_bytes_8_to_12() {
        let got = decode(&h(0x1111_1111, 0x2222_2222, 0x3333_3333), &[0x0A, 0x00]).unwrap();
        assert_eq!(got.error_id, 0x3333_3333);
        assert_eq!(got.err_code, 0);
    }

    #[test]
    fn server_time_comes_from_show_msg_field_3() {
        let with = |show: Vec<u8>| decode(&h(0, 7, 0), &root(&[show, num(4, T)])).unwrap();
        assert_eq!(with(bytes(3, &num(3, T))).server_time, Some(T));
        assert_eq!(
            decode(&h(0, 7, 0), &root(&[num(4, T), num(5, 5211)]))
                .unwrap()
                .server_time,
            None
        );
        assert_eq!(with(bytes(3, &[])).server_time, None);
        assert_eq!(with(bytes(3, &num(3, 0))).server_time, None);
        assert_eq!(with(bytes(3, &[0x18])).server_time, None); // cut short
        assert_eq!(with(bytes(3, &bytes(3, b"x"))).server_time, None); // wrong wire type
        assert_eq!(with(num(3, T)).server_time, None); // show_msg as a varint
    }
}
