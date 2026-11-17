//! Replays real captures (`tests/fixtures/*.capture.log`, made with the app's
//! debug "Raw Capture" and kept out of git) through the pipeline. With no
//! capture present this passes vacuously; it pins what the real capture must give:
//! the chats counted by an independent decode.
//!
//! The sanitised corpus (S7, #270) is in git and replayed on every run: Kade's
//! capture of 2026-10-03 after `examples/sanitise_capture.rs`, with the real
//! traffic's segmentation, retransmits and history but no real name, uid, line or
//! address.

use resonance_core::capture::{replay, ChatPipeline, PipelineAction};
use resonance_core::protocol::framing::{Frame, FrameAssembler, FrameKind};
use resonance_types::ChatMessage;
use std::fs;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/oct03.sanitised.capture.log"
);

#[test]
fn the_sanitised_corpus_decodes_the_same_every_time() {
    let text = fs::read_to_string(CORPUS).expect("the sanitised corpus is in git");
    let packets = replay(&text);
    // Only a sanitised capture goes here: every address is in the documentation
    // ranges the sanitiser moves them to (a raw capture renamed by mistake fails).
    for packet in &packets {
        assert_eq!(packet[12..15], [198, 51, 100], "server address");
        assert_eq!(packet[16..19], [192, 0, 2], "client address");
    }
    let mut pipeline = ChatPipeline::new();
    let mut pid = 0;
    let chats: Vec<ChatMessage> = packets
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
            PipelineAction::EmitNewMessage(chat, _, _) => Some(chat),
            PipelineAction::UpdateBlockedMessage(_) | PipelineAction::OwnPlayer(..) => None,
        })
        .collect();
    assert_eq!((packets.len(), chats.len()), (2164, 380));
    // Every line as the app shows it: a decoder change shows here as a diff.
    let table: String = chats
        .iter()
        .map(|m| {
            format!(
                "{:?} {} uid {} lv {} seq {} t {} {}
",
                m.channel, m.nickname, m.uid, m.level, m.sequence_id, m.timestamp, m.message
            )
        })
        .collect();
    insta::assert_snapshot!(table);
}

/// The frames of the corpus's one stream, in order: the packets' payloads in TCP sequence
/// (a retransmit or the overlap of a resent segment is dropped; the corpus has no gaps).
fn corpus_frames() -> Vec<Frame> {
    let text = fs::read_to_string(CORPUS).expect("the sanitised corpus is in git");
    let mut assembler = FrameAssembler::new();
    let mut next: Option<u32> = None;
    let mut frames = Vec::new();
    for packet in replay(&text) {
        let ip_len = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
        let ihl = usize::from(packet[0] & 0x0F) * 4;
        let tcp = &packet[ihl..];
        let seq = u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]);
        let data = usize::from(tcp[12] >> 4) * 4;
        let mut payload = &packet[ihl + data..ip_len.min(packet.len())];
        let mut at = seq;
        if let Some(n) = next {
            let ahead = seq.wrapping_sub(n) as i32;
            assert!(ahead <= 0, "the corpus has no sequence gap");
            let seen = ahead.unsigned_abs() as usize;
            if seen >= payload.len() {
                continue;
            }
            payload = &payload[seen..];
            at = n;
        }
        next = Some(at.wrapping_add(payload.len() as u32));
        frames.extend(assembler.push(payload));
    }
    frames
}

/// FNV-1a, 64 bit.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// What "the existing chat decoding stays byte-for-byte the same" is compared against: every
/// frame that is chat, history or world state, with its root's length and hash.
#[test]
fn the_frames_the_app_reads_are_pinned() {
    let table: String = corpus_frames()
        .iter()
        .filter(|f| {
            matches!(
                f.kind,
                FrameKind::Live | FrameKind::History | FrameKind::State
            )
        })
        .map(|f| format!("{:?} {} {:016x}\n", f.kind, f.root.len(), fnv1a(&f.root)))
        .collect();
    insta::assert_snapshot!(table);
}

#[test]
fn real_captures_replay_without_panicking() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        let is_capture = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".capture.log"));
        if !is_capture {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let mut pipeline = ChatPipeline::new();
        let (mut packets, mut chats) = (0, 0);
        let mut pid = 0;
        for packet in replay(&text) {
            packets += 1;
            chats += pipeline
                .feed_network_packet(
                    &packet,
                    |_| false,
                    || {
                        pid += 1;
                        pid
                    },
                    || {},
                )
                .iter()
                .filter(|a| matches!(a, PipelineAction::EmitNewMessage(..)))
                .count();
        }
        println!("{}: {packets} packets -> {chats} chats", path.display());
        assert!(packets > 0, "{} has no usable packets", path.display());
        if path.ends_with("real1.capture.log") {
            // 33 live lines + 127 history lines the client had not sent us
            // live, counted by an independent decode (python + zstandard) of
            // the same file with the app's dedup key (uid, time, id).
            assert_eq!((packets, chats), (256, 160));
        }
    }
}
