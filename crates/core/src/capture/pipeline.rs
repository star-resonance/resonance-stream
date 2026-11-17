use crate::capture::message_processor::{MessageProcessor, ProcessAction, DEFAULT_DEDUP_CAPACITY};
use crate::capture::stream_tracker::{StreamKey, StreamTracker};
use crate::game_clients::{ClientFact, Endpoint, Roster};
use crate::protocol::framing::FrameKind;
use crate::protocol::own_player::{own_player, OwnPlayer};
use crate::protocol::parser::{history_pipeline, parsing_pipeline, Port5003Event};
use crate::text::normalize_emotes;
use etherparse::{NetHeaders, PacketHeaders, TransportHeader};
use resonance_types::ChatMessage;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The shortest time between two looks for idle connections.
const EXPIRE_EVERY: Duration = Duration::from_secs(10);

pub enum PipelineAction {
    UpdateBlockedMessage(ChatMessage),
    /// A new message, its origin and the connection it came in on (`None`: `feed_chat`, not captured).
    EmitNewMessage(ChatMessage, Origin, Option<Endpoint>),
    /// A connection's own player (uid, guild), read from world state: `Some` when its first
    /// record arrives and again whenever it changes; `None` when the connection is gone (closed,
    /// or idle past `CLIENT_IDLE_TIMEOUT`).
    OwnPlayer(Endpoint, Option<OwnPlayer>),
}

/// Where a new message came from. The guild relay sends only live lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A live chat frame (`0x0002`).
    Live,
    /// The channel history the server sends at log-in or on a channel switch.
    History,
    /// Fed through `feed_chat` (`--replay-chat`), not captured.
    Replay,
}

pub struct ChatPipeline {
    tracker: StreamTracker,
    processor: MessageProcessor,
    keep_unknown_fields: bool,
    /// When idle connections were last looked for (at most every `EXPIRE_EVERY`).
    last_expire: Option<Instant>,
    /// Which connection is which character (read before the duplicate check).
    roster: Roster,
}

impl ChatPipeline {
    pub fn new() -> Self {
        Self {
            tracker: StreamTracker::new(),
            processor: MessageProcessor::new(),
            keep_unknown_fields: false,
            last_expire: None,
            roster: Roster::new(),
        }
    }

    /// Keep the raw bytes of fields the parser does not understand on each
    /// message (a reverse-engineering aid). Off by default: they are never
    /// displayed, and would otherwise be cloned, stored and sent to the UI.
    pub fn set_keep_unknown_fields(&mut self, keep: bool) {
        self.keep_unknown_fields = keep;
    }

    /// Teaches the duplicate check messages shown before (e.g. history
    /// reloaded from disk), so the server re-sending them is ignored. The
    /// cache grows to hold all of them, plus the default room for new lines.
    pub fn remember(&mut self, messages: &[ChatMessage]) {
        self.processor
            .grow_to(messages.len() + DEFAULT_DEDUP_CAPACITY);
        for message in messages {
            self.processor.commit_new_message(message);
        }
    }

    /// The connections and their characters' nicknames, when they changed since the last call.
    pub fn take_clients(&mut self) -> Option<Vec<ClientFact>> {
        self.roster.take_changed(Instant::now())
    }

    /// Drops the connections idle past `CLIENT_IDLE_TIMEOUT` at `now`; each is reported as
    /// `OwnPlayer(conn, None)`.
    pub fn expire(&mut self, now: Instant) -> Vec<PipelineAction> {
        self.roster.expire(now);
        self.gone_actions()
    }

    fn gone_actions(&mut self) -> Vec<PipelineAction> {
        self.roster
            .take_gone()
            .into_iter()
            .map(|conn| PipelineAction::OwnPlayer(conn, None))
            .collect()
    }

    /// 100% Pure Logic: Takes raw network bytes and returns UI Actions.
    pub fn feed_network_packet(
        &mut self,
        packet: &[u8],
        is_blocked: impl Fn(u64) -> bool,
        mut assign_pid: impl FnMut() -> u64,
        mut feed_watchdog: impl FnMut(),
    ) -> Vec<PipelineAction> {
        let mut actions = Vec::new();

        // 1. Guard clauses: Fail fast if it's not the exact IPv4/TCP/5003 packet we want
        let Ok(headers) = PacketHeaders::from_ip_slice(packet) else {
            return actions;
        };
        let Some(TransportHeader::Tcp(tcp)) = headers.transport else {
            return actions;
        };
        if tcp.source_port != 5003 {
            return actions;
        }

        let now = Instant::now();
        if let Some(NetHeaders::Ipv4(ipv4, _)) = &headers.net {
            if tcp.fin || tcp.rst {
                self.roster.forget(Endpoint {
                    addr: ipv4.destination,
                    port: tcp.destination_port,
                });
            }
        }
        // A silent client is noticed on the next 5003 packet of any client.
        if self
            .last_expire
            .is_none_or(|last| now.saturating_duration_since(last) >= EXPIRE_EVERY)
        {
            self.last_expire = Some(now);
            self.roster.expire(now);
        }
        actions.extend(self.gone_actions());

        let payload = headers.payload.slice();
        if payload.is_empty() {
            return actions;
        }

        // Only IPv4 is decoded, so only IPv4 traffic proves the game is talking.
        let Some(NetHeaders::Ipv4(ipv4, _)) = headers.net else {
            return actions;
        };

        feed_watchdog();

        // 2. Build the unique TCP connection key (both ends)
        let mut stream_key: StreamKey = [0u8; 12];
        stream_key[0..4].copy_from_slice(&ipv4.source);
        stream_key[4..6].copy_from_slice(&tcp.source_port.to_be_bytes());
        stream_key[6..10].copy_from_slice(&ipv4.destination);
        stream_key[10..12].copy_from_slice(&tcp.destination_port.to_be_bytes());

        let local = Endpoint {
            addr: ipv4.destination,
            port: tcp.destination_port,
        };
        self.roster.see_connection(local, now);

        // 3. Assemble fragmented bytes into complete frames (retransmits dropped)
        let frames = self
            .tracker
            .process_bytes(stream_key, tcp.sequence_number, payload);

        // 4. Process the fully assembled frames
        for frame in frames {
            let (events, origin) = match frame.kind {
                FrameKind::Live => (parsing_pipeline(&frame.root), Origin::Live),
                FrameKind::History => (history_pipeline(&frame.root), Origin::History),
                FrameKind::State => {
                    if let Some(player) = own_player(&frame.root) {
                        if self.roster.own_record(local, player, now) {
                            actions.push(PipelineAction::OwnPlayer(local, Some(player)));
                        }
                    }
                    continue;
                }
            };
            for event in events {
                // Guard clause for the event loop using let-else
                let Port5003Event::Chat(chat) = event;
                actions.extend(self.feed(chat, origin, Some(local), &is_blocked, &mut assign_pid));
            }
        }

        actions
    }

    /// One decoded chat message through the rules every message gets: unknown
    /// fields dropped (unless asked for), stickers/emotes in their display form,
    /// then the duplicate and block rules. `None`: a duplicate, nothing to do.
    /// Also how a replayed message (`--replay-chat`) enters, so it is treated
    /// exactly like a captured one.
    pub fn feed_chat(
        &mut self,
        chat: ChatMessage,
        is_blocked: impl Fn(u64) -> bool,
        assign_pid: impl FnMut() -> u64,
    ) -> Option<PipelineAction> {
        self.feed(chat, Origin::Replay, None, is_blocked, assign_pid)
    }

    fn feed(
        &mut self,
        mut chat: ChatMessage,
        origin: Origin,
        conn: Option<Endpoint>,
        is_blocked: impl Fn(u64) -> bool,
        mut assign_pid: impl FnMut() -> u64,
    ) -> Option<PipelineAction> {
        // Before the duplicate check: a line dropped as a duplicate still names its sender.
        self.roster.see_line(chat.uid, &chat.nickname);
        if !self.keep_unknown_fields {
            chat.unknown_fields = HashMap::new();
        }
        // Display form of stickers/emotes, before anything (the
        // translator included) sees the text.
        chat.message = normalize_emotes(&chat.message);

        // 5. Apply duplicate and blocking rules
        match self.processor.process(&mut chat, &is_blocked) {
            ProcessAction::IgnoreDuplicate => None,
            ProcessAction::UpdateBlockedMessage => Some(PipelineAction::UpdateBlockedMessage(chat)),
            ProcessAction::EmitNewMessage => {
                chat.pid = assign_pid();
                self.processor.commit_new_message(&chat);
                Some(PipelineAction::EmitNewMessage(chat, origin, conn))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use etherparse::PacketBuilder;
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};
    use std::collections::HashMap;

    /// `[len][type][16-byte header][root]`: a live chat frame as the game sends it.
    fn frame(root: &[u8]) -> Vec<u8> {
        frame_of(0x0002, 16, root)
    }

    fn frame_of(ty: u16, header: usize, payload: &[u8]) -> Vec<u8> {
        let len = (6 + header + payload.len()) as u32;
        [
            &len.to_be_bytes()[..],
            &ty.to_be_bytes(),
            &vec![0u8; header],
            payload,
        ]
        .concat()
    }

    /// `[len][0x8003][12-byte header][zstd(root)]`: the channel history.
    fn history_frame(root: &[u8]) -> Vec<u8> {
        frame_of(
            0x8003,
            12,
            &compress_to_vec(root, CompressionLevel::Fastest),
        )
    }

    #[test]
    fn test_full_chat_pipeline() {
        let mut pipeline = ChatPipeline::new();
        let blocked_users: HashMap<u64, String> = HashMap::new();

        let mut mock_pid_counter = 1;
        let mut assign_pid = || {
            let pid = mock_pid_counter;
            mock_pid_counter += 1;
            pid
        };

        // 1. Construct the raw Protobuf payload for a complete BPSR Chat Message
        // Includes Session ID (Sequence ID), Sender Info (Nickname & UID), and Message Text
        let tcp_root = vec![
            0x0A, // Protobuf Root Tag
            0x14, // Root Length: 20 bytes
            // --- ChatPayload Wrapper (Tag 18 -> 0x12) ---
            0x12, 0x12, // Tag 18, Length 18
            // --- Field 1: Session ID (Tag 8 -> 0x08) ---
            0x08, 0xE7, 0x07, // Sequence ID = 999
            // --- Field 2: SenderInfo (Tag 18 -> 0x12) ---
            0x12, 0x07, // Tag 18, Length 7
            0x08, 0x64, // UID (Tag 8) = 100
            0x12, 0x03, 0x42, 0x6F, 0x62, // Nickname (Tag 18) = "Bob"
            // --- Field 4: Message (Tag 34 -> 0x22) ---
            0x22, 0x04, // Tag 34, Length 4
            0x1A, 0x02, 0x48, 0x69, // Text (Tag 26) = "Hi"
        ];
        let tcp_payload = frame(&tcp_root);

        // 2. Wrap it in valid IPv4 and TCP headers
        let builder =
            PacketBuilder::ipv4([192, 168, 1, 1], [192, 168, 1, 2], 64).tcp(5003, 12345, 1, 0);

        let mut fake_network_packet = Vec::<u8>::new();
        builder
            .write(&mut fake_network_packet, &tcp_payload)
            .unwrap();

        // 3. Feed the forged packet into our pure pipeline
        let actions = pipeline.feed_network_packet(
            &fake_network_packet,
            |uid| blocked_users.contains_key(&uid),
            &mut assign_pid,
            || {}, // Pass an empty closure for the test!
        );

        // 4. Assert that the pipeline correctly extracted ALL fields!
        assert_eq!(actions.len(), 1);
        if let PipelineAction::EmitNewMessage(chat, _, _) = &actions[0] {
            assert_eq!(chat.pid, 1, "Local UI PID should be 1");
            assert_eq!(
                chat.sequence_id, 999,
                "Sequence ID from packet should be 999"
            );
            assert_eq!(chat.nickname, "Bob", "Nickname should be Bob");
            assert_eq!(chat.uid, 100, "UID should be 100");
            assert_eq!(chat.message, "Hi", "Message should be Hi");
        } else {
            panic!("Pipeline failed to emit a new message.");
        }
    }
    fn tcp_from_5003(seq: u32, payload: &[u8]) -> Vec<u8> {
        let builder =
            PacketBuilder::ipv4([192, 168, 1, 1], [192, 168, 1, 2], 64).tcp(5003, 12345, seq, 0);
        let mut out = Vec::new();
        builder.write(&mut out, payload).unwrap();
        out
    }

    /// Like `tcp_from_5003`, but to a chosen client (address and port).
    fn tcp_from_5003_to(client: [u8; 4], port: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
        let builder = PacketBuilder::ipv4([192, 168, 1, 1], client, 64).tcp(5003, port, seq, 0);
        let mut out = Vec::new();
        builder.write(&mut out, payload).unwrap();
        out
    }

    #[test]
    fn two_clients_on_one_server_are_reassembled_independently() {
        // W1: the stream key was server address + port only, so the halves of
        // two clients' split frames were joined into one buffer and lost.
        let mut pipeline = ChatPipeline::new();
        let hello = chat_segment(1, "Hello");
        let world = chat_segment(2, "World");
        let (a1, a2) = hello.split_at(10);
        let (b1, b2) = world.split_at(10);
        let order = [
            ([10, 0, 0, 1], 40000, 1_000, a1),
            ([10, 0, 0, 2], 40001, 9_000, b1),
            ([10, 0, 0, 1], 40000, 1_000 + a1.len() as u32, a2),
            ([10, 0, 0, 2], 40001, 9_000 + b1.len() as u32, b2),
        ];
        let mut got = Vec::new();
        for (client, port, seq, part) in order {
            for action in pipeline.feed_network_packet(
                &tcp_from_5003_to(client, port, seq, part),
                |_| false,
                || 1,
                || {},
            ) {
                if let PipelineAction::EmitNewMessage(chat, _, _) = action {
                    got.push(chat.message);
                }
            }
        }
        assert_eq!(got, ["Hello", "World"]);
    }

    /// Frame + root { field 4: { tag 0x1A text } } -- a "Me" message.
    fn me_segment(text: &str) -> Vec<u8> {
        let mut block = vec![0x1A, text.len() as u8];
        block.extend_from_slice(text.as_bytes());
        let mut root = vec![0x22, block.len() as u8];
        root.extend(block);
        let mut seg = vec![0x0A, root.len() as u8];
        seg.extend(root);
        frame(&seg)
    }

    fn emitted(pipeline: &mut ChatPipeline, segments: &[Vec<u8>]) -> Vec<ChatMessage> {
        let blocked: HashMap<u64, String> = HashMap::new();
        let mut pid = 0;
        let mut seq = 1_000u32;
        let mut out = Vec::new();
        for seg in segments {
            let at = seq;
            seq = seq.wrapping_add(seg.len() as u32);
            let actions = pipeline.feed_network_packet(
                &tcp_from_5003(at, seg),
                |uid| blocked.contains_key(&uid),
                || {
                    pid += 1;
                    pid
                },
                || {},
            );
            for action in actions {
                if let PipelineAction::EmitNewMessage(chat, _, _) = action {
                    out.push(chat);
                }
            }
        }
        out
    }

    #[test]
    fn every_me_message_is_emitted() {
        let mut pipeline = ChatPipeline::new();
        let got = emitted(&mut pipeline, &[me_segment("one"), me_segment("two")]);
        let texts: Vec<_> = got.iter().map(|c| c.message.as_str()).collect();
        assert_eq!(texts, ["one", "two"]);
    }

    /// A chat as both the live and the history frames carry it:
    /// { 1: seq, 2: sender "Bob" (uid 100), 4: message }.
    fn chat_payload(seq: u8, text: &str) -> Vec<u8> {
        let mut msg = vec![0x1A, text.len() as u8];
        msg.extend_from_slice(text.as_bytes());
        let sender = [0x08, 0x64, 0x12, 0x03, b'B', b'o', b'b'];
        let mut payload = vec![0x08, seq, 0x12, sender.len() as u8];
        payload.extend_from_slice(&sender);
        payload.extend([0x22, msg.len() as u8]);
        payload.extend(msg);
        payload
    }

    /// Frame + root { field 2: chat }.
    fn chat_segment(seq: u8, text: &str) -> Vec<u8> {
        let payload = chat_payload(seq, text);
        let mut root = vec![0x12, payload.len() as u8];
        root.extend(payload);
        let mut seg = vec![0x0A, root.len() as u8];
        seg.extend(root);
        frame(&seg)
    }

    /// Compressed history frame, root { 3: WORLD, 5: chat, 5: chat, ... }; the
    /// lines are given oldest first and sent newest first, as the server does.
    fn history_segment(lines: &[(u8, &str)]) -> Vec<u8> {
        let mut fields = vec![0x18, 0x01];
        for (seq, text) in lines.iter().rev() {
            let payload = chat_payload(*seq, text);
            fields.extend([0x2A, payload.len() as u8]);
            fields.extend(payload);
        }
        let mut root = vec![0x0A, fields.len() as u8];
        root.extend(fields);
        history_frame(&root)
    }

    fn texts(chats: &[ChatMessage]) -> Vec<&str> {
        chats.iter().map(|c| c.message.as_str()).collect()
    }

    #[test]
    fn coalesced_messages_are_all_emitted() {
        // Regression (review B2): only the last message of a segment survived.
        let mut pipeline = ChatPipeline::new();
        let segment = [chat_segment(1, "Hello"), chat_segment(2, "World")].concat();
        let got = emitted(&mut pipeline, &[segment]);
        assert_eq!(texts(&got), ["Hello", "World"]);
    }

    #[test]
    fn message_split_across_segments_is_reassembled() {
        // Regression (review B2): the first half never ended on a boundary and was dropped.
        let mut pipeline = ChatPipeline::new();
        let whole = chat_segment(1, "Hello");
        let (a, b) = whole.split_at(10);
        let got = emitted(&mut pipeline, &[a.to_vec(), b.to_vec()]);
        assert_eq!(texts(&got), ["Hello"]);
    }

    #[test]
    fn split_and_coalesced_together() {
        let mut pipeline = ChatPipeline::new();
        let stream = [
            chat_segment(1, "one"),
            chat_segment(2, "two"),
            chat_segment(3, "three"),
        ]
        .concat();
        let (a, rest) = stream.split_at(12);
        let (b, c) = rest.split_at(30);
        let got = emitted(&mut pipeline, &[a.to_vec(), b.to_vec(), c.to_vec()]);
        assert_eq!(texts(&got), ["one", "two", "three"]);
    }

    #[test]
    fn emotes_are_normalized_before_leaving_the_pipeline() {
        let mut pipeline = ChatPipeline::new();
        let got = emitted(
            &mut pipeline,
            &[
                chat_segment(1, "hi<sprite=3>"),
                chat_segment(2, "emojiPic=9"),
            ],
        );
        assert_eq!(texts(&got), ["hi[이모지]", "[스티커]"]);
    }

    #[test]
    fn reloaded_history_beyond_the_default_capacity_is_still_remembered() {
        // W4: the duplicate cache held 4096 signatures, so with bigger chat
        // limits the oldest reloaded lines were forgotten and shown again.
        let count = DEFAULT_DEDUP_CAPACITY as u64 + 1000;
        let history: Vec<ChatMessage> = (1..=count)
            .map(|i| ChatMessage {
                uid: 7,
                timestamp: i,
                sequence_id: i,
                pid: i,
                ..Default::default()
            })
            .collect();
        let mut pipeline = ChatPipeline::new();
        pipeline.remember(&history);

        let mut oldest = history[0].clone();
        let action = pipeline.processor.process(&mut oldest, &|_| false);
        assert!(matches!(action, ProcessAction::IgnoreDuplicate));
    }

    #[test]
    fn ipv6_traffic_does_not_keep_the_watchdog_alive() {
        // W7: the watchdog was fed before the IPv4 check, so IPv6 packets
        // from port 5003 (which are never decoded) read as "game traffic".
        use std::cell::Cell;
        let mut pipeline = ChatPipeline::new();
        let fed = Cell::new(0);
        let mut feed = |packet: &[u8]| {
            pipeline.feed_network_packet(packet, |_| false, || 1, || fed.set(fed.get() + 1))
        };

        let mut v6 = Vec::new();
        PacketBuilder::ipv6([1; 16], [2; 16], 64)
            .tcp(5003, 12345, 1, 0)
            .write(&mut v6, &chat_segment(1, "hi"))
            .unwrap();
        assert!(feed(&v6).is_empty());
        assert_eq!(fed.get(), 0, "IPv6 must not feed the watchdog");

        feed(&tcp_from_5003(1, &chat_segment(1, "hi")));
        assert_eq!(fed.get(), 1, "IPv4 port-5003 traffic still does");
    }

    #[test]
    fn blocklist_is_consulted_only_for_chat_messages() {
        use std::cell::Cell;
        let mut pipeline = ChatPipeline::new();
        let lookups = Cell::new(0);
        let is_blocked = |uid: u64| {
            lookups.set(lookups.get() + 1);
            uid == 100
        };

        // Other traffic: not port 5003, and a 5003 segment that is not chat.
        let other = PacketBuilder::ipv4([1, 1, 1, 1], [2, 2, 2, 2], 64).tcp(443, 5000, 1, 0);
        let mut https = Vec::new();
        other.write(&mut https, b"hello").unwrap();
        pipeline.feed_network_packet(&https, &is_blocked, || 1, || {});
        pipeline.feed_network_packet(
            &tcp_from_5003(1, &[1, 2, 3, 4, 5]),
            &is_blocked,
            || 1,
            || {},
        );
        assert_eq!(lookups.get(), 0);

        // One chat message from uid 100: one lookup, and it is marked blocked.
        let actions = pipeline.feed_network_packet(
            &tcp_from_5003(6, &chat_segment(1, "hi")), // after the 5 bytes above
            &is_blocked,
            || 1,
            || {},
        );
        assert_eq!(lookups.get(), 1);
        match &actions[..] {
            [PipelineAction::EmitNewMessage(chat, _, _)] => assert!(chat.is_blocked),
            _ => panic!("expected one new message"),
        }
    }

    #[test]
    fn remembered_messages_are_not_emitted_again() {
        let mut pipeline = ChatPipeline::new();
        // What a previous run saw and saved: seq 1 "Hello" from uid 100.
        let first = emitted(&mut pipeline, &[chat_segment(1, "Hello")]);
        let mut fresh = ChatPipeline::new();
        fresh.remember(&first);
        let again = emitted(
            &mut fresh,
            &[chat_segment(1, "Hello"), chat_segment(2, "New")],
        );
        assert_eq!(texts(&again), ["New"]);
    }

    /// Frame + a chat whose payload carries an unknown field (tag 0x28 = field 5).
    fn chat_with_unknown_field() -> Vec<u8> {
        let payload = [
            0x08, 0x01, // session id 1
            0x12, 0x02, 0x08, 0x64, // sender uid 100
            0x28, 0x07, // unknown field 5 = 7
            0x22, 0x04, 0x1A, 0x02, b'H', b'i', // message "Hi"
        ];
        let mut root = vec![0x12, payload.len() as u8];
        root.extend_from_slice(&payload);
        let mut seg = vec![0x0A, root.len() as u8];
        seg.extend(root);
        frame(&seg)
    }

    #[test]
    fn unknown_fields_are_dropped_unless_asked_for() {
        let mut pipeline = ChatPipeline::new();
        let got = emitted(&mut pipeline, &[chat_with_unknown_field()]);
        assert_eq!(got.len(), 1);
        assert!(got[0].unknown_fields.is_empty());

        let mut pipeline = ChatPipeline::new();
        pipeline.set_keep_unknown_fields(true);
        let got = emitted(&mut pipeline, &[chat_with_unknown_field()]);
        assert!(!got[0].unknown_fields.is_empty());
    }

    #[test]
    fn history_lines_are_emitted_oldest_first() {
        let mut pipeline = ChatPipeline::new();
        let got = emitted(&mut pipeline, &[history_segment(&[(1, "old"), (2, "new")])]);
        let texts: Vec<_> = got.iter().map(|c| c.message.as_str()).collect();
        assert_eq!(texts, ["old", "new"]);
        assert_eq!(got[0].channel, resonance_types::Channel::World);
        assert_eq!(got[0].nickname, "Bob");
    }

    #[test]
    fn history_lines_already_seen_live_are_not_shown_again() {
        let mut pipeline = ChatPipeline::new();
        let got = emitted(
            &mut pipeline,
            &[
                chat_segment(2, "seen live"),
                history_segment(&[(1, "missed"), (2, "seen live"), (3, "also missed")]),
                history_segment(&[(1, "missed"), (2, "seen live"), (3, "also missed")]),
            ],
        );
        let texts: Vec<_> = got.iter().map(|c| c.message.as_str()).collect();
        assert_eq!(texts, ["seen live", "missed", "also missed"]);
    }

    #[test]
    fn each_new_message_says_whether_it_came_live_from_history_or_from_a_replay() {
        let mut pipeline = ChatPipeline::new();
        let mut got = Vec::new();
        let live = chat_segment(2, "live");
        let history = history_segment(&[(1, "missed")]);
        for (seq, segment) in [(1_000, &live), (1_000 + live.len() as u32, &history)] {
            for action in
                pipeline.feed_network_packet(&tcp_from_5003(seq, segment), |_| false, || 1, || {})
            {
                if let PipelineAction::EmitNewMessage(chat, origin, _) = action {
                    got.push((chat.message, origin));
                }
            }
        }
        if let Some(PipelineAction::EmitNewMessage(chat, origin, _)) =
            pipeline.feed_chat(chat_from(7, 1000, 1, "replayed"), |_| false, || 1)
        {
            got.push((chat.message, origin));
        }
        assert_eq!(
            got,
            [
                ("live".to_string(), Origin::Live),
                ("missed".to_string(), Origin::History),
                ("replayed".to_string(), Origin::Replay),
            ]
        );
    }

    // ---- feed_chat: one decoded message through the same rules as a captured one ----

    fn chat_from(uid: u64, timestamp: u64, sequence_id: u64, text: &str) -> ChatMessage {
        ChatMessage {
            uid,
            timestamp,
            sequence_id,
            message: text.to_string(),
            nickname: "tester".to_string(),
            ..ChatMessage::default()
        }
    }

    fn emitted_chat(action: Option<PipelineAction>) -> ChatMessage {
        match action {
            Some(PipelineAction::EmitNewMessage(chat, _, _)) => chat,
            Some(PipelineAction::UpdateBlockedMessage(_)) => {
                panic!("expected a new message, got a block update")
            }
            Some(PipelineAction::OwnPlayer(..)) => {
                panic!("expected a new message, got an own-player record")
            }
            None => panic!("expected a new message, got nothing"),
        }
    }

    #[test]
    fn a_fed_chat_is_emitted_with_the_next_pid() {
        let mut pipeline = ChatPipeline::new();
        let mut next = 40;
        let chat = emitted_chat(pipeline.feed_chat(
            chat_from(7, 1000, 1, "hello"),
            |_| false,
            || {
                next += 1;
                next
            },
        ));
        assert_eq!(
            (chat.pid, chat.message.as_str(), chat.is_blocked),
            (41, "hello", false)
        );
    }

    #[test]
    fn a_fed_chat_seen_before_is_ignored() {
        let mut pipeline = ChatPipeline::new();
        let first = pipeline.feed_chat(chat_from(7, 1000, 1, "hello"), |_| false, || 1);
        assert!(first.is_some());
        let again = pipeline.feed_chat(chat_from(7, 1000, 1, "hello"), |_| false, || 2);
        assert!(again.is_none());
        // The same sender, words and send time under another sequence id is a second game
        // client's copy: ignored here, not later in the app (review R-3).
        assert!(pipeline
            .feed_chat(chat_from(7, 1000, 2, "hello"), |_| false, || 3)
            .is_none());
        // Other words in the same second are another message.
        assert!(pipeline
            .feed_chat(chat_from(7, 1000, 3, "hello!"), |_| false, || 4)
            .is_some());
    }

    #[test]
    fn a_second_clients_copy_of_a_reloaded_message_is_ignored() {
        // History reloaded from disk teaches both keys, so the server's or a second client's
        // re-send of a message already shown is not shown twice.
        let mut pipeline = ChatPipeline::new();
        pipeline.remember(&[chat_from(7, 1000, 1, "hello")]);
        assert!(pipeline
            .feed_chat(chat_from(7, 1000, 99, "hello"), |_| false, || 1)
            .is_none());
        assert!(pipeline
            .feed_chat(chat_from(7, 1001, 100, "hello"), |_| false, || 2)
            .is_some());
    }

    #[test]
    fn a_fed_chat_from_a_blocked_user_is_emitted_blocked() {
        let mut pipeline = ChatPipeline::new();
        let chat =
            emitted_chat(pipeline.feed_chat(chat_from(7, 1000, 1, "hi"), |uid| uid == 7, || 1));
        assert!(chat.is_blocked);
    }

    #[test]
    fn blocking_a_user_after_the_fact_updates_the_row_it_already_has() {
        let mut pipeline = ChatPipeline::new();
        let first = emitted_chat(pipeline.feed_chat(chat_from(7, 1000, 1, "hi"), |_| false, || 5));
        assert_eq!(first.pid, 5);
        match pipeline.feed_chat(chat_from(7, 1000, 1, "hi"), |uid| uid == 7, || 6) {
            Some(PipelineAction::UpdateBlockedMessage(chat)) => {
                assert_eq!((chat.pid, chat.is_blocked), (5, true));
            }
            _ => panic!("expected a block update for pid 5"),
        }
    }

    #[test]
    fn a_fed_chat_gets_emotes_normalized_and_unknown_fields_dropped() {
        let mut pipeline = ChatPipeline::new();
        let mut chat = chat_from(7, 1000, 1, "hi<sprite=3>");
        chat.unknown_fields.insert("9".to_string(), vec![1, 2]);
        let got = emitted_chat(pipeline.feed_chat(chat, |_| false, || 1));
        assert_eq!(got.message, "hi[이모지]");
        assert!(got.unknown_fields.is_empty());
    }

    #[test]
    fn messages_without_identity_are_never_duplicates() {
        // timestamp 0 and sequence id 0: the game's own "me" lines
        let mut pipeline = ChatPipeline::new();
        assert!(pipeline
            .feed_chat(chat_from(7, 0, 0, "a"), |_| false, || 1)
            .is_some());
        assert!(pipeline
            .feed_chat(chat_from(7, 0, 0, "a"), |_| false, || 2)
            .is_some());
    }

    #[test]
    fn a_retransmitted_frame_is_emitted_once() {
        let mut pipeline = ChatPipeline::new();
        let seg = chat_segment(1, "once");
        let mut out = Vec::new();
        for packet in [tcp_from_5003(1_000, &seg), tcp_from_5003(1_000, &seg)] {
            out.extend(pipeline.feed_network_packet(&packet, |_| false, || 1, || {}));
        }
        assert_eq!(out.len(), 1);
    }

    // ---- world state: the player's own record ----

    fn varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn pb_int(field: u8, v: u64) -> Vec<u8> {
        [vec![field << 3], varint(v)].concat()
    }

    fn pb_msg(field: u8, body: &[u8]) -> Vec<u8> {
        [
            vec![(field << 3) | 2],
            varint(body.len() as u64),
            body.to_vec(),
        ]
        .concat()
    }

    /// A compressed `0x8002` frame holding the player's own record.
    fn own_record_frame(uid: u64, guild: Option<u64>) -> Vec<u8> {
        let mut record = [
            pb_int(1, uid),
            pb_msg(3, &[pb_int(1, uid), pb_int(2, uid)].concat()),
        ]
        .concat();
        if let Some(id) = guild {
            record.extend(pb_msg(13, &pb_int(1, id)));
        }
        let root = pb_msg(1, &pb_msg(1, &record));
        frame_of(
            0x8002,
            16,
            &compress_to_vec(&root[..], CompressionLevel::Fastest),
        )
    }

    fn own_players(actions: &[PipelineAction]) -> Vec<(Endpoint, Option<OwnPlayer>)> {
        actions
            .iter()
            .filter_map(|a| match a {
                PipelineAction::OwnPlayer(c, p) => Some((*c, *p)),
                _ => None,
            })
            .collect()
    }

    fn single_client() -> Endpoint {
        Endpoint {
            addr: [192, 168, 1, 2],
            port: 12345,
        }
    }

    fn ep(port: u16) -> Endpoint {
        Endpoint {
            addr: [10, 0, 0, 5],
            port,
        }
    }

    #[test]
    fn the_own_record_is_reported_once_and_again_when_it_changes() {
        let mut pipeline = ChatPipeline::new();
        let mut seq = 1u32;
        let mut feed = |frame: Vec<u8>| {
            let packet = tcp_from_5003(seq, &frame);
            seq += frame.len() as u32;
            pipeline.feed_network_packet(&packet, |_| false, || 1, || {})
        };
        let in_guild = OwnPlayer {
            uid: 424_242,
            guild_id: Some(77_001),
        };

        let first = feed(own_record_frame(424_242, Some(77_001)));
        assert_eq!(own_players(&first), [(single_client(), Some(in_guild))]);
        assert_eq!(first.len(), 1, "a record is not a chat line");

        let again = feed(own_record_frame(424_242, Some(77_001)));
        assert!(again.is_empty(), "the same record is not reported twice");

        let left = feed(own_record_frame(424_242, None));
        assert_eq!(
            own_players(&left),
            [(
                single_client(),
                Some(OwnPlayer {
                    uid: 424_242,
                    guild_id: None
                })
            )]
        );
    }

    fn p7() -> OwnPlayer {
        OwnPlayer {
            uid: 7,
            guild_id: Some(100),
        }
    }

    fn p9() -> OwnPlayer {
        OwnPlayer {
            uid: 9,
            guild_id: Some(200),
        }
    }

    /// Two clients' state frames, alternating three times (the old single slot flapped on each).
    fn interleaved(
        pipeline: &mut ChatPipeline,
        seqs: &mut HashMap<u16, u32>,
    ) -> Vec<PipelineAction> {
        let mut out = Vec::new();
        for _ in 0..3 {
            out.extend(feed_to(
                pipeline,
                seqs,
                51000,
                &own_record_frame(7, Some(100)),
            ));
            out.extend(feed_to(
                pipeline,
                seqs,
                51001,
                &own_record_frame(9, Some(200)),
            ));
        }
        out
    }

    // F-GR 3
    #[test]
    fn two_clients_report_each_record_once_however_they_interleave() {
        let mut pipeline = ChatPipeline::new();
        let mut seqs = HashMap::new();
        let actions = interleaved(&mut pipeline, &mut seqs);
        assert_eq!(
            own_players(&actions),
            [(ep(51000), Some(p7())), (ep(51001), Some(p9()))]
        );
        let moved = feed_to(
            &mut pipeline,
            &mut seqs,
            51000,
            &own_record_frame(7, Some(101)),
        );
        assert_eq!(
            own_players(&moved),
            [(
                ep(51000),
                Some(OwnPlayer {
                    uid: 7,
                    guild_id: Some(101)
                })
            )]
        );
        let b_again = feed_to(
            &mut pipeline,
            &mut seqs,
            51001,
            &own_record_frame(9, Some(200)),
        );
        assert!(own_players(&b_again).is_empty());
    }

    // F-GR 4
    #[test]
    fn a_line_carries_the_connection_it_came_in_on() {
        let mut pipeline = ChatPipeline::new();
        let mut seqs = HashMap::new();
        interleaved(&mut pipeline, &mut seqs);
        let conn_of = |actions: Vec<PipelineAction>| match actions.as_slice() {
            [PipelineAction::EmitNewMessage(_, Origin::Live, conn)] => *conn,
            _ => panic!("expected one live line"),
        };
        let on_b = feed_to(
            &mut pipeline,
            &mut seqs,
            51001,
            &chat_segment_by(9, "Bob", 1, "b"),
        );
        assert_eq!(conn_of(on_b), Some(ep(51001)));
        let on_a = feed_to(
            &mut pipeline,
            &mut seqs,
            51000,
            &chat_segment_by(7, "Aria", 2, "a"),
        );
        assert_eq!(conn_of(on_a), Some(ep(51000)));
        match pipeline.feed_chat(chat_from(7, 1000, 3, "replayed"), |_| false, || 1) {
            Some(PipelineAction::EmitNewMessage(_, Origin::Replay, None)) => {}
            _ => panic!("a replayed line has no connection"),
        }
    }

    /// A packet from port 5003 to `10.0.0.5:port` with no payload and the given flag.
    fn bare_packet(port: u16, seq: u32, flag: &str) -> Vec<u8> {
        let builder =
            PacketBuilder::ipv4([192, 168, 1, 1], [10, 0, 0, 5], 64).tcp(5003, port, seq, 0);
        let builder = match flag {
            "fin" => builder.fin(),
            "rst" => builder.rst(),
            _ => builder.ack(1),
        };
        let mut out = Vec::new();
        builder.write(&mut out, &[]).unwrap();
        out
    }

    // F-GR 6
    #[test]
    fn a_closed_connection_reports_its_record_gone() {
        for flag in ["fin", "rst"] {
            let mut pipeline = ChatPipeline::new();
            let mut seqs = HashMap::new();
            interleaved(&mut pipeline, &mut seqs);
            let _ = pipeline.take_clients();
            let gone = pipeline.feed_network_packet(
                &bare_packet(51000, 9_999, flag),
                |_| false,
                || 1,
                || {},
            );
            assert_eq!(own_players(&gone), [(ep(51000), None)], "{flag}");
            let ack = pipeline.feed_network_packet(
                &bare_packet(51000, 9_999, "ack"),
                |_| false,
                || 1,
                || {},
            );
            assert!(ack.is_empty());
            let clients = pipeline.take_clients().expect("the list changed");
            assert!(clients.iter().all(|c| c.local != ep(51000)));
            let b = feed_to(
                &mut pipeline,
                &mut seqs,
                51001,
                &own_record_frame(9, Some(200)),
            );
            assert!(own_players(&b).is_empty(), "b is untouched");
        }
    }

    // F-GR 7
    #[test]
    fn idle_connections_report_their_records_gone() {
        let mut pipeline = ChatPipeline::new();
        let mut seqs = HashMap::new();
        interleaved(&mut pipeline, &mut seqs);
        let later = Instant::now()
            + crate::game_clients::CLIENT_IDLE_TIMEOUT
            + std::time::Duration::from_secs(1);
        let gone = pipeline.expire(later);
        assert_eq!(own_players(&gone), [(ep(51000), None), (ep(51001), None)]);
        let again = interleaved(&mut pipeline, &mut seqs);
        assert_eq!(
            own_players(&again),
            [(ep(51000), Some(p7())), (ep(51001), Some(p9()))]
        );
    }

    // ---- the roster of game clients (F-MC-2) ----

    /// A live chat frame from sender `uid` named `nick`: { 1: seq, 2: { 1: uid, 2: nick }, 4: text }.
    fn chat_segment_by(uid: u64, nick: &str, seq: u8, text: &str) -> Vec<u8> {
        let mut msg = vec![0x1A, text.len() as u8];
        msg.extend_from_slice(text.as_bytes());
        let sender = [pb_int(1, uid), pb_msg(2, nick.as_bytes())].concat();
        let mut payload = vec![0x08, seq];
        payload.extend(pb_msg(2, &sender));
        payload.extend([0x22, msg.len() as u8]);
        payload.extend(msg);
        let root = pb_msg(1, &pb_msg(2, &payload));
        frame(&root)
    }

    /// Feeds `frame` as sent to `client`:`port`; `seqs` keeps each connection's sequence number.
    fn feed_to(
        pipeline: &mut ChatPipeline,
        seqs: &mut HashMap<u16, u32>,
        port: u16,
        frame: &[u8],
    ) -> Vec<PipelineAction> {
        let seq = seqs.entry(port).or_insert(1);
        let packet = tcp_from_5003_to([10, 0, 0, 5], port, *seq, frame);
        *seq += frame.len() as u32;
        pipeline.feed_network_packet(&packet, |_| false, || 1, || {})
    }

    fn named(port: u16, nick: Option<&str>) -> crate::game_clients::ClientFact {
        crate::game_clients::ClientFact {
            local: crate::game_clients::Endpoint {
                addr: [10, 0, 0, 5],
                port,
            },
            nickname: nick.map(str::to_string),
        }
    }

    #[test]
    fn a_line_dropped_as_a_duplicate_still_names_the_client() {
        let mut pipeline = ChatPipeline::new();
        let mut seqs = HashMap::new();
        // The history from disk already holds Bob's line (uid 9, sequence 5).
        pipeline.remember(&[chat_from(9, 0, 5, "hello")]);
        feed_to(&mut pipeline, &mut seqs, 51001, &own_record_frame(9, None));
        let actions = feed_to(
            &mut pipeline,
            &mut seqs,
            51001,
            &chat_segment_by(9, "Bob", 5, "hello"),
        );
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, PipelineAction::EmitNewMessage(..))),
            "the line is a duplicate"
        );
        assert_eq!(
            pipeline.take_clients(),
            Some(vec![named(51001, Some("Bob"))])
        );
    }

    #[test]
    fn two_clients_are_named_each_by_its_own_character() {
        let mut pipeline = ChatPipeline::new();
        let mut seqs = HashMap::new();
        for _ in 0..2 {
            feed_to(&mut pipeline, &mut seqs, 51000, &own_record_frame(7, None));
            feed_to(&mut pipeline, &mut seqs, 51001, &own_record_frame(9, None));
        }
        feed_to(
            &mut pipeline,
            &mut seqs,
            51000,
            &chat_segment_by(7, "Aria", 1, "a"),
        );
        feed_to(
            &mut pipeline,
            &mut seqs,
            51001,
            &chat_segment_by(9, "Bob", 2, "b"),
        );
        assert_eq!(
            pipeline.take_clients(),
            Some(vec![named(51000, Some("Aria")), named(51001, Some("Bob"))])
        );
        // The same state frames again, alternating (the own-player slot flaps): nothing changes.
        feed_to(&mut pipeline, &mut seqs, 51000, &own_record_frame(7, None));
        feed_to(&mut pipeline, &mut seqs, 51001, &own_record_frame(9, None));
        assert_eq!(pipeline.take_clients(), None);
    }
}
