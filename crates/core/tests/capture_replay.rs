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
            PipelineAction::EmitNewMessage(chat) => Some(chat),
            PipelineAction::UpdateBlockedMessage(_) => None,
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
                .filter(|a| matches!(a, PipelineAction::EmitNewMessage(_)))
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
