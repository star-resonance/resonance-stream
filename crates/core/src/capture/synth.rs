//! The inverse of the parser: builds the bytes the game server would send for a chat line, so
//! a test can put a chat through the real socket path (frame -> TCP -> sniffer -> parser ->
//! dedup -> `packet-event`) with no game. The layout is the one `protocol::parser` reads:
//! root `{1: channel code, 2: chat}`, chat `{1: sequence, 2: sender, 3: time, 4: message}`,
//! sender `{1: uid, 2: nickname, 5: level}`, message `{3: text}`; framing as in
//! `protocol::framing` (type `0x0002`, a 16-byte inner header).

use resonance_types::{Channel, ChatMessage};

/// What a channel is called on the wire (the inverse of `Channel::known_code`).
fn channel_code(channel: Channel) -> u64 {
    match channel {
        Channel::World => 1,
        Channel::Local => 2,
        Channel::Party => 3,
        Channel::Guild => 4,
        Channel::Beginner => 9,
    }
}

fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7F) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// A varint field: `tag` is the full tag byte (`field << 3`).
fn varint_field(tag: u8, value: u64, out: &mut Vec<u8>) {
    out.push(tag);
    varint(value, out);
}

/// A length-delimited field.
fn bytes_field(tag: u8, body: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    varint(body.len() as u64, out);
    out.extend_from_slice(body);
}

/// The chat line as one live frame (`[len][0x0002][16-byte header][root]`).
pub fn live_chat_frame(chat: &ChatMessage) -> Vec<u8> {
    let mut sender = Vec::new();
    varint_field(0x08, chat.uid, &mut sender);
    bytes_field(0x12, chat.nickname.as_bytes(), &mut sender);
    varint_field(0x28, chat.level, &mut sender);

    let mut message = Vec::new();
    bytes_field(0x1A, chat.message.as_bytes(), &mut message);

    let mut payload = Vec::new();
    varint_field(0x08, chat.sequence_id, &mut payload);
    bytes_field(0x12, &sender, &mut payload);
    varint_field(0x18, chat.timestamp, &mut payload);
    bytes_field(0x22, &message, &mut payload);

    let mut fields = Vec::new();
    varint_field(0x08, channel_code(chat.channel), &mut fields);
    bytes_field(0x12, &payload, &mut fields);

    let mut root = vec![0x0A];
    varint(fields.len() as u64, &mut root);
    root.extend(fields);

    let mut frame = vec![0u8; 4];
    frame.extend(0x0002u16.to_be_bytes());
    frame.extend([0xEE; 16]);
    frame.extend(root);
    let len = frame.len() as u32;
    frame[..4].copy_from_slice(&len.to_be_bytes());
    frame
}

/// A channel's recent lines as one history frame (`[len][0x8003][12-byte header][zstd(root)]`,
/// root `{3: channel code, 5: chat, 5: chat, ...}`), `newest_first` as the server sends them.
pub fn history_frame(_channel: Channel, _newest_first: &[ChatMessage]) -> Vec<u8> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{ChatPipeline, PipelineAction};
    use etherparse::PacketBuilder;

    fn chat(channel: Channel, nickname: &str, level: u64, text: &str, seq: u64) -> ChatMessage {
        ChatMessage {
            channel,
            nickname: nickname.into(),
            uid: 1_234_567_890_123,
            level,
            sequence_id: seq,
            timestamp: 1_700_000_000,
            message: text.into(),
            ..Default::default()
        }
    }

    /// The frame in a TCP segment from port 5003, as the sniffer would see it.
    fn packet(payload: &[u8], seq: u32) -> Vec<u8> {
        let mut out = Vec::new();
        PacketBuilder::ipv4([172, 65, 0, 1], [10, 0, 0, 2], 64)
            .tcp(5003, 40000, seq, 0)
            .write(&mut out, payload)
            .unwrap();
        out
    }

    fn through_the_pipeline(frames: &[Vec<u8>]) -> Vec<ChatMessage> {
        let mut pipeline = ChatPipeline::new();
        let mut pid = 0;
        let mut seq = 1000u32;
        let mut out = Vec::new();
        for frame in frames {
            let actions = pipeline.feed_network_packet(
                &packet(frame, seq),
                |_| false,
                || {
                    pid += 1;
                    pid
                },
                || {},
            );
            seq = seq.wrapping_add(frame.len() as u32);
            for action in actions {
                if let PipelineAction::EmitNewMessage(m) = action {
                    out.push(m);
                }
            }
        }
        out
    }

    #[test]
    fn a_built_frame_comes_out_of_the_real_pipeline_as_the_same_chat() {
        for channel in Channel::ALL {
            let sent = chat(
                channel,
                "ミナト",
                60,
                "こんにちは、よろしくお願いします！",
                7,
            );
            let got = through_the_pipeline(&[live_chat_frame(&sent)]);
            assert_eq!(got.len(), 1, "{channel:?}");
            let got = &got[0];
            assert_eq!(got.channel, channel);
            assert_eq!(got.nickname, sent.nickname);
            assert_eq!(got.uid, sent.uid);
            assert_eq!(got.level, sent.level);
            assert_eq!(got.sequence_id, sent.sequence_id);
            assert_eq!(got.timestamp, sent.timestamp);
            assert_eq!(got.message, sent.message);
        }
    }

    #[test]
    fn long_lines_and_big_numbers_need_real_varints() {
        let text = "あ".repeat(300); // 900 bytes: the lengths are two-byte varints
        let mut sent = chat(Channel::Guild, "Kenji", 1, &text, 300);
        sent.uid = u64::from(u32::MAX) + 5;
        let got = through_the_pipeline(&[live_chat_frame(&sent)]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, text);
        assert_eq!(got[0].uid, sent.uid);
        assert_eq!(got[0].sequence_id, 300);
    }

    #[test]
    fn a_built_history_frame_comes_out_oldest_first() {
        let newest = chat(Channel::Guild, "ミナト", 60, "二番目", 8);
        let older = chat(Channel::Guild, "Kenji", 12, "一番目", 7);
        let got = through_the_pipeline(&[history_frame(
            Channel::Guild,
            &[newest.clone(), older.clone()],
        )]);
        let got: Vec<(Channel, &str, u64, &str)> = got
            .iter()
            .map(|m| {
                (
                    m.channel,
                    m.nickname.as_str(),
                    m.sequence_id,
                    m.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (Channel::Guild, "Kenji", 7, "一番目"),
                (Channel::Guild, "ミナト", 8, "二番目"),
            ]
        );
    }

    #[test]
    fn frames_sent_back_to_back_all_arrive() {
        let frames: Vec<Vec<u8>> = (1..=5u64)
            .map(|n| live_chat_frame(&chat(Channel::World, "a", 60, &format!("line {n}"), n)))
            .collect();
        let texts: Vec<String> = through_the_pipeline(&frames)
            .into_iter()
            .map(|m| m.message)
            .collect();
        assert_eq!(texts, ["line 1", "line 2", "line 3", "line 4", "line 5"]);
    }
}
