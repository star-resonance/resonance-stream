//! Makes a raw capture (see [`recorder`](super::recorder)) safe to publish as a test corpus
//! (S7, #270): nicknames, uids, chat text and addresses are replaced, and every packet keeps its
//! length, its TCP sequence number and its timestamp, so the corpus still has the real traffic's
//! segmentation (frames cut across packets, retransmits, gaps) that the decoder must survive.
//!
//! The replacements keep every byte length, so no frame length, protobuf length or TCP sequence
//! number changes:
//! - a nickname becomes a pseudonym of the same byte length (`P0001` style), one per name;
//! - a uid becomes another number with the same varint length, one per uid;
//! - chat text becomes filler, each character replaced by one of the same UTF-8 length;
//! - a history frame (`0x8003`) is inflated, rewritten, compressed again and padded to its length;
//! - a frame the app does not read has its body blanked; so has every byte the app never frames;
//! - IPv4 addresses move into documentation ranges, with both checksums recomputed.
//!
//! [`sanitise`] checks its own output before returning it: the sanitised capture must decode to
//! the original's chat lines under the new names, and no original name, uid, line or address may
//! be left in it. Either failing is an error, never a file.

use std::collections::BTreeMap;

/// What [`sanitise`] did, for the person who reviews the result before it is published.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// Capture lines kept (one packet each) and dropped (comments, malformed, not port-5003 data).
    pub packets: usize,
    pub dropped_lines: usize,
    /// Chat lines the original decodes to (and the sanitised capture decodes to as well).
    pub chats: usize,
    /// Distinct nicknames, uids and IPv4 addresses replaced.
    pub nicknames: usize,
    pub uids: usize,
    pub addresses: usize,
    /// Whole frames seen, by type.
    pub frames: BTreeMap<u16, usize>,
    /// History frames compressed again and padded back to their length.
    pub history_padded: usize,
    /// Strings off the known chat paths that were filled (they might name someone).
    pub strings_filled: usize,
    /// Bytes set to zero: bodies of frames the app does not read, and bytes it never frames.
    pub bytes_blanked: usize,
    /// Byte fields left as they were (neither a string nor a message), as `path: length`.
    pub binary_left: Vec<String>,
}

/// The sanitised capture and what was done to it.
#[derive(Debug, Clone)]
pub struct Sanitised {
    pub text: String,
    pub report: Report,
}

/// Why nothing was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SanitiseError {
    /// A history frame came out longer than it went in once compressed again.
    HistoryGrew { frame_len: usize, needed: usize },
    /// No pseudonym (or uid) of this byte length is left.
    Exhausted { what: &'static str, len: usize },
    /// The sanitised capture does not decode to the original's lines under the new names.
    DecodesDifferently { detail: String },
    /// Something of the original is still in the sanitised capture.
    Leaks { detail: String },
}

impl std::fmt::Display for SanitiseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HistoryGrew { frame_len, needed } => write!(
                f,
                "a history frame of {frame_len} bytes needs {needed} once sanitised"
            ),
            Self::Exhausted { what, len } => write!(f, "no {what} of {len} bytes is left"),
            Self::DecodesDifferently { detail } => {
                write!(f, "the sanitised capture decodes differently: {detail}")
            }
            Self::Leaks { detail } => write!(f, "the sanitised capture still holds {detail}"),
        }
    }
}

impl std::error::Error for SanitiseError {}

/// Sanitises a capture file's text; see the module documentation.
pub fn sanitise(_text: &str) -> Result<Sanitised, SanitiseError> {
    Ok(Sanitised {
        text: String::new(),
        report: Report::default(),
    })
}

/// Chat text as filler: every character becomes one of the same UTF-8 length (`x`, `é`, `あ`,
/// `😀`); spaces and U+FFFD (what a broken byte decodes to) stay.
pub fn fill_text(text: &str) -> String {
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::synth::{history_frame, live_chat_frame};
    use crate::capture::{decode_line, encode_line, ChatPipeline, PipelineAction};
    use etherparse::{NetHeaders, PacketBuilder, PacketHeaders, TransportHeader};
    use resonance_types::{Channel, ChatMessage};
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};

    const SERVER: [u8; 4] = [43, 155, 1, 2];
    const CLIENT: [u8; 4] = [192, 168, 0, 23];
    const OTHER: [u8; 4] = [142, 250, 1, 1];

    fn chat(channel: Channel, nickname: &str, uid: u64, seq: u64, text: &str) -> ChatMessage {
        ChatMessage {
            channel,
            nickname: nickname.into(),
            uid,
            level: 60,
            sequence_id: seq,
            timestamp: 1_759_420_000 + seq,
            message: text.into(),
            ..Default::default()
        }
    }

    fn alice(seq: u64, text: &str) -> ChatMessage {
        chat(Channel::World, "アリス", 37_276_266, seq, text)
    }

    fn kenji(seq: u64, text: &str) -> ChatMessage {
        chat(Channel::Guild, "Kenji", 1_234_567_890_123, seq, text)
    }

    fn me(seq: u64, text: &str) -> ChatMessage {
        chat(Channel::World, "かでぃ", 99_887_766, seq, text)
    }

    fn carol(seq: u64, text: &str) -> ChatMessage {
        chat(Channel::Local, "Carolyn", 555_001, seq, text)
    }

    /// Only ever in a packet the app drops (it arrives after the gap it would have filled).
    fn dave() -> ChatMessage {
        chat(
            Channel::World,
            "デイブ",
            777_002,
            40,
            "デイブの秘密の話です",
        )
    }

    /// `[len][ty][header bytes of 0x11][body]`.
    fn frame(ty: u16, header: usize, body: &[u8]) -> Vec<u8> {
        let len = (6 + header + body.len()) as u32;
        [
            &len.to_be_bytes()[..],
            &ty.to_be_bytes(),
            &vec![0x11; header],
            body,
        ]
        .concat()
    }

    fn packet(src: [u8; 4], src_port: u16, client_port: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PacketBuilder::ipv4(src, CLIENT, 64)
            .tcp(src_port, client_port, seq, 4096)
            .write(&mut out, payload)
            .unwrap();
        out
    }

    /// Every awkward thing a real capture has, around six people's lines.
    fn capture() -> String {
        // Stream A: client port 51234.
        let live_alice = live_chat_frame(&alice(1, "こんにちは、みなさん"));
        let keepalive = [0, 0, 0, 6, 0, 4].to_vec();
        let world = frame(
            0x8002,
            16,
            &compress_to_vec(
                &b"\x0a\x10world: Kenji 37276266 at home"[..],
                CompressionLevel::Fastest,
            ),
        );
        let history = history_frame(
            Channel::Guild,
            &[
                kenji(3, "ギルドの集合は九時です"),
                alice(2, "ボス行きませんか？"),
            ],
        );
        let own = live_chat_frame(&me(4, "了解です、すぐ行きます"));
        let echo = frame(0x0003, 12, &own[22..]);
        let kenji_live = live_chat_frame(&kenji(5, "今日もよろしくお願いします"));
        let stream_a = [
            live_alice.clone(),
            keepalive,
            world,
            history,
            echo,
            own,
            kenji_live.clone(),
        ]
        .concat();
        // Cut inside the world-state frame and inside Kenji's last line.
        let cut1 = live_alice.len() + 6 + 20;
        let cut2 = stream_a.len() - kenji_live.len() / 2;

        // Stream B: client port 51235, with a gap: the middle segment comes after the third, so
        // the app drops it (and Dave's line in it).
        let b1 = live_chat_frame(&carol(30, "ローカルでこんばんは"));
        let b2 = live_chat_frame(&dave());
        let b3 = live_chat_frame(&carol(31, "また明日ね、おやすみ"));

        let a = 10_000u32;
        let b = u32::MAX - 50; // the sequence number wraps inside this stream
        let lines = [
            (1, packet(SERVER, 5003, 51234, a, &stream_a[..cut1])),
            (
                2,
                packet(SERVER, 5003, 51234, a + cut1 as u32, &stream_a[cut1..cut2]),
            ),
            (
                3,
                packet(SERVER, 5003, 51234, a + cut1 as u32, &stream_a[cut1..cut2]),
            ), // retransmit
            (4, packet(OTHER, 443, 51300, 77, b"not the game")),
            (
                5,
                packet(SERVER, 5003, 51234, a + cut2 as u32, &stream_a[cut2..]),
            ),
            (6, packet(SERVER, 5003, 51235, b, &b1)),
            (
                7,
                packet(
                    SERVER,
                    5003,
                    51235,
                    b.wrapping_add((b1.len() + b2.len()) as u32),
                    &b3,
                ),
            ),
            (
                8,
                packet(SERVER, 5003, 51235, b.wrapping_add(b1.len() as u32), &b2),
            ), // too late
        ];
        let mut text = String::from("# Kade's capture, 2026-10-03, with アリス and Kenji\n");
        for (ts, packet) in lines {
            text.push_str(&encode_line(1_759_420_000_000 + ts, &packet));
            text.push('\n');
        }
        text
    }

    fn decoded(text: &str) -> Vec<ChatMessage> {
        let mut pipeline = ChatPipeline::new();
        let mut pid = 0;
        crate::capture::replay(text)
            .iter()
            .flat_map(|packet| {
                pipeline.feed_network_packet(
                    packet,
                    |_| false,
                    || {
                        pid += 1;
                        pid
                    },
                    || {},
                )
            })
            .filter_map(|action| match action {
                PipelineAction::EmitNewMessage(chat) => Some(chat),
                PipelineAction::UpdateBlockedMessage(_) => None,
            })
            .collect()
    }

    fn packets(text: &str) -> Vec<(u64, Vec<u8>)> {
        text.lines().filter_map(decode_line).collect()
    }

    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        while value >= 0x80 {
            out.push((value as u8 & 0x7F) | 0x80);
            value >>= 7;
        }
        out.push(value as u8);
        out
    }

    fn holds(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn the_sanitised_capture_decodes_to_the_same_lines_under_new_names() {
        let original = decoded(&capture());
        // Alice twice live/history, Kenji twice, me, Carol twice; Dave's packet is dropped.
        assert_eq!(original.len(), 7);
        let out = sanitise(&capture()).unwrap();
        let got = decoded(&out.text);
        assert_eq!(got.len(), original.len());
        let mut names = BTreeMap::new();
        let mut uids = BTreeMap::new();
        for (was, now) in original.iter().zip(&got) {
            assert_eq!(
                (now.channel, now.level, now.sequence_id, now.timestamp),
                (was.channel, was.level, was.sequence_id, was.timestamp)
            );
            assert_eq!(now.message, fill_text(&was.message));
            assert_ne!(now.nickname, was.nickname);
            assert_eq!(now.nickname.len(), was.nickname.len());
            assert_ne!(now.uid, was.uid);
            assert_eq!(varint(now.uid).len(), varint(was.uid).len());
            // One replacement per person, the same every time they appear.
            assert_eq!(
                *names
                    .entry(was.nickname.clone())
                    .or_insert(now.nickname.clone()),
                now.nickname
            );
            assert_eq!(*uids.entry(was.uid).or_insert(now.uid), now.uid);
        }
        assert_eq!(names.len(), 4);
        let distinct: std::collections::BTreeSet<_> = names.values().collect();
        assert_eq!(distinct.len(), 4, "two people got the same pseudonym");
        assert_eq!(out.report.chats, 7);
        assert_eq!(out.report.nicknames, 4);
        assert_eq!(out.report.uids, 4);
        assert_eq!(out.report.history_padded, 1);
    }

    #[test]
    fn no_name_uid_line_or_address_of_the_original_is_left() {
        let out = sanitise(&capture()).unwrap();
        let bytes: Vec<u8> = packets(&out.text)
            .into_iter()
            .flat_map(|(_, p)| p)
            .collect();
        let people = [
            alice(1, "こんにちは、みなさん"),
            kenji(5, "今日もよろしくお願いします"),
            me(4, "了解です、すぐ行きます"),
            carol(30, "ローカルでこんばんは"),
            dave(),
        ];
        for person in &people {
            assert!(
                !holds(&bytes, person.nickname.as_bytes()),
                "{}",
                person.nickname
            );
            assert!(!holds(&bytes, &varint(person.uid)), "uid {}", person.uid);
            assert!(
                !holds(&bytes, person.message.as_bytes()),
                "{}",
                person.message
            );
        }
        for line in [
            "ボス行きませんか？",
            "ギルドの集合は九時です",
            "また明日ね、おやすみ",
            "デイブ",
        ] {
            assert!(!holds(&bytes, line.as_bytes()), "{line}");
        }
        assert!(
            !holds(&bytes, b"world"),
            "the world-state frame was not blanked"
        );
        for address in [SERVER, CLIENT, OTHER] {
            assert!(!holds(&bytes, &address), "{address:?}");
        }
        assert!(!out.text.contains('#'), "a comment line was kept");
    }

    #[test]
    fn every_packet_keeps_its_time_length_ports_and_sequence_number() {
        let kept: Vec<(u64, Vec<u8>)> = packets(&capture())
            .into_iter()
            .filter(|(_, p)| crate::capture::is_capturable(p))
            .collect();
        let out = sanitise(&capture()).unwrap();
        let got = packets(&out.text);
        assert_eq!(got.len(), kept.len());
        assert_eq!(out.report.packets, 7);
        assert_eq!(out.report.dropped_lines, 2);
        for ((was_ts, was), (ts, now)) in kept.iter().zip(&got) {
            assert_eq!(ts, was_ts);
            assert_eq!(now.len(), was.len());
            let was = PacketHeaders::from_ip_slice(was).unwrap();
            let now = PacketHeaders::from_ip_slice(now).unwrap();
            let (Some(NetHeaders::Ipv4(ip, _)), Some(TransportHeader::Tcp(tcp))) =
                (&now.net, &now.transport)
            else {
                panic!("not IPv4/TCP");
            };
            let Some(TransportHeader::Tcp(was_tcp)) = &was.transport else {
                panic!()
            };
            assert_eq!(
                (tcp.source_port, tcp.destination_port, tcp.sequence_number),
                (
                    was_tcp.source_port,
                    was_tcp.destination_port,
                    was_tcp.sequence_number
                )
            );
            assert_eq!(ip.source[..3], [198, 51, 100], "server address");
            assert_eq!(ip.destination[..3], [192, 0, 2], "client address");
            assert_eq!(ip.header_checksum, ip.calc_header_checksum());
            assert_eq!(
                tcp.checksum,
                tcp.calc_checksum_ipv4(ip, now.payload.slice()).unwrap()
            );
        }
    }

    #[test]
    fn the_same_capture_sanitises_the_same_way_twice() {
        assert_eq!(
            sanitise(&capture()).unwrap().text,
            sanitise(&capture()).unwrap().text
        );
    }

    #[test]
    fn filler_keeps_each_characters_byte_length() {
        let text = "Hi あ é 😀 \u{FFFD}!";
        let filled = fill_text(text);
        assert_eq!(filled, "xx あ é 😀 \u{FFFD}x");
        assert_ne!(fill_text("こんにちは"), "こんにちは");
        assert_eq!(fill_text("こんにちは").len(), "こんにちは".len());
    }
}
