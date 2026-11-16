#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;
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
        assert_eq!(build_send_chat(WireChannel::World, "hi", 1), Ok(G1.to_vec()));
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
            assert_eq!(u32::from_be_bytes([f[0], f[1], f[2], f[3]]) as usize, f.len());
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
        assert_eq!(u32::from_be_bytes([f[0], f[1], f[2], f[3]]) as usize, f.len());
        let _ = vec![0u8; 0];
    }
}
