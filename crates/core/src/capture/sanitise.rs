//! Makes a raw capture (from [`RawCaptureWriter`](super::RawCaptureWriter)) safe to publish as
//! a test corpus (S7, #270): nicknames, uids, chat text and addresses are replaced, and every
//! packet keeps its length, its TCP sequence number and its timestamp, so the corpus still has the
//! real traffic's segmentation (frames cut across packets, retransmits, gaps) that the decoder
//! must survive.
//!
//! The replacements keep every byte length, so no frame length, protobuf length or TCP sequence
//! number changes:
//! - a nickname becomes a pseudonym of the same byte length (`P0001` style), one per name;
//! - a uid becomes another number with the same varint length, one per uid, wherever it appears
//!   (a field, a packed list), except as a chat line's sequence or time. Only the uids of the
//!   people who speak are known: any other number stays, and [`Report::numbers_left`] lists the
//!   large ones by path for the person who reviews the result;
//! - a string off the chat paths is filled like chat text (a known nickname gets its pseudonym);
//! - chat text becomes filler, each character replaced by one of the same UTF-8 length;
//! - a history frame (`0x8003`) is inflated, rewritten, compressed again and padded to its length;
//! - a frame the app does not read (world state) has its body blanked; so has every inner header
//!   and every byte the app never frames;
//! - IPv4 addresses move into documentation ranges, with both checksums recomputed.
//!
//! [`sanitise`] checks its own output before returning it: the sanitised capture must decode to
//! the original's chat lines under the new names, and no original name, uid, line or address may
//! be left in it (by bytes, for those of 4 bytes or more). Either failing is an error, never a
//! file.

use super::stream_tracker::{StreamKey, StreamTracker};
use super::{decode_line, encode_line, is_capturable, replay, ChatPipeline, PipelineAction};
use crate::protocol::compression::decompress;
use crate::protocol::decoder::{read_tag, read_varint, Fields, Value};
use crate::protocol::framing::{
    has_history_lines, inner_header, FrameAssembler, FrameKind, COMPRESSED, PREFIX_LEN,
    TYPE_KEEPALIVE, TYPE_OWN_LINE,
};
use crate::protocol::parser::{history_pipeline, parsing_pipeline, Port5003Event};
use etherparse::{NetHeaders, PacketHeaders, TransportHeader};
use resonance_types::ChatMessage;
use ruzstd::encoding::{compress_to_vec, CompressionLevel};
use std::collections::{BTreeMap, HashMap};

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
    /// Known uids replaced inside packed lists of varints.
    pub packed_uids: usize,
    /// Bytes set to zero: bodies of frames the app does not read, and bytes it never frames.
    pub bytes_blanked: usize,
    /// Byte fields left as they were (neither a string nor a message), as `path: length`.
    pub binary_left: Vec<String>,
    /// Numbers of 10 000 or more left as they were (not a known uid, not a chat line's sequence or
    /// time), by path with their range: an id of someone who never spoke would be one of them.
    pub numbers_left: Vec<String>,
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
pub fn sanitise(text: &str) -> Result<Sanitised, SanitiseError> {
    let (lines, dropped_lines) = read_lines(text);
    let mut streams = Streams::default();
    for (index, line) in lines.iter().enumerate() {
        streams.deliver(index, line);
    }

    let mut people = People::default();
    // First pass: learn every sender's name and uid, so a uid or name met off the sender path
    // before its owner speaks is replaced too. The rewritten bytes are thrown away.
    let mut scratch = streams.clone();
    scratch.rewrite(&mut people, &mut Report::default())?;
    people.binary_left.clear();
    people.numbers_left.clear();
    let mut report = Report::default();
    streams.rewrite(&mut people, &mut report)?;

    let mut addresses = Addresses::default();
    let mut out = String::new();
    let mut packets = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let mut packet = line.packet.clone();
        let blanked = streams.fill_payload(index, line, &mut packet);
        report.bytes_blanked += blanked;
        addresses.rewrite(&mut packet)?;
        fix_checksums(&mut packet);
        out.push_str(&encode_line(line.ts, &packet));
        out.push('\n');
        packets.push(packet);
    }

    report.packets = lines.len();
    report.dropped_lines = dropped_lines;
    report.nicknames = people.names.len();
    report.uids = people.uids.len();
    report.addresses = addresses.map.len();
    report.binary_left = people
        .binary_left
        .iter()
        .map(|(path, (count, bytes))| format!("{path}: {count} fields, {bytes} bytes"))
        .collect();
    report.numbers_left = people
        .numbers_left
        .iter()
        .map(|(path, (count, min, max))| format!("{path}: {count} numbers, {min} to {max}"))
        .collect();
    let was = decode_lines(text);
    report.chats = check_decodes_the_same(text, &was, &out, &people)?;
    check_nothing_left(&was, &streams, &packets, &people, &addresses)?;
    Ok(Sanitised { text: out, report })
}

/// Chat text as filler: every character becomes one of the same UTF-8 length (`x`, `é`, `あ`,
/// `😀`); spaces and U+FFFD (what a broken byte decodes to) stay.
pub fn fill_text(text: &str) -> String {
    text.chars().map(filler).collect()
}

fn filler(c: char) -> char {
    match c {
        ' ' | '\u{FFFD}' => c,
        _ => match c.len_utf8() {
            1 => 'x',
            2 => 'é',
            3 => 'あ',
            _ => '😀',
        },
    }
}

/// [`fill_text`] on raw bytes, in place: bytes that are not UTF-8 stay as they are.
fn fill_bytes(bytes: &mut [u8]) {
    let mut out = Vec::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        let mut buf = [0u8; 4];
        for c in chunk.valid().chars() {
            out.extend_from_slice(filler(c).encode_utf8(&mut buf).as_bytes());
        }
        out.extend_from_slice(chunk.invalid());
    }
    bytes.copy_from_slice(&out);
}

// --- the capture's lines ----------------------------------------------------------------------

/// One kept capture line: an IPv4/TCP packet from port 5003 with data.
struct Line {
    ts: u64,
    packet: Vec<u8>,
    key: StreamKey,
    seq: u32,
    /// Where the TCP payload sits in `packet`, and its length.
    payload_at: usize,
    payload_len: usize,
}

/// The lines [`replay`](super::replay) would feed the app, and how many others were dropped.
fn read_lines(text: &str) -> (Vec<Line>, usize) {
    let mut lines = Vec::new();
    let mut dropped = 0;
    for raw in text.lines() {
        let line = (!raw.starts_with('#'))
            .then(|| decode_line(raw))
            .flatten()
            .filter(|(_, packet)| is_capturable(packet))
            .and_then(|(ts, packet)| line_of(ts, packet));
        match line {
            Some(line) => lines.push(line),
            None if raw.trim().is_empty() => {}
            None => dropped += 1,
        }
    }
    (lines, dropped)
}

fn line_of(ts: u64, packet: Vec<u8>) -> Option<Line> {
    let headers = PacketHeaders::from_ip_slice(&packet).ok()?;
    let (Some(NetHeaders::Ipv4(ip, _)), Some(TransportHeader::Tcp(tcp))) =
        (&headers.net, &headers.transport)
    else {
        return None;
    };
    let payload = headers.payload.slice();
    let mut key: StreamKey = [0; 12];
    key[0..4].copy_from_slice(&ip.source);
    key[4..6].copy_from_slice(&tcp.source_port.to_be_bytes());
    key[6..10].copy_from_slice(&ip.destination);
    key[10..12].copy_from_slice(&tcp.destination_port.to_be_bytes());
    let payload_at = payload.as_ptr() as usize - packet.as_ptr() as usize;
    let (seq, payload_len) = (tcp.sequence_number, payload.len());
    Some(Line {
        ts,
        key,
        seq,
        payload_at,
        payload_len,
        packet,
    })
}

// --- the streams, as the app reads them ---------------------------------------------------------

/// The bytes of a connection the app delivers without a gap, and the pieces they came in.
#[derive(Clone)]
struct Run {
    /// Stream position of the first byte (positions count from the connection's first packet).
    start: i64,
    bytes: Vec<u8>,
    pieces: Vec<usize>,
    /// Decompressed history roots after the rewrite, for the leak check.
    roots: Vec<Vec<u8>>,
}

#[derive(Clone, Default)]
struct Stream {
    next_seq: Option<u32>,
    next_pos: i64,
    runs: Vec<Run>,
}

/// Every connection, in the order it first appears, read the way
/// [`StreamTracker`](super::stream_tracker::StreamTracker) reads it: bytes seen before are
/// dropped, a gap starts a new run.
#[derive(Clone, Default)]
struct Streams {
    order: Vec<StreamKey>,
    streams: HashMap<StreamKey, Stream>,
    /// Stream position of each line's first payload byte.
    line_pos: HashMap<usize, i64>,
}

impl Streams {
    fn deliver(&mut self, index: usize, line: &Line) {
        if !self.streams.contains_key(&line.key) {
            self.order.push(line.key);
        }
        let stream = self.streams.entry(line.key).or_default();
        let payload = &line.packet[line.payload_at..line.payload_at + line.payload_len];
        let ahead = match stream.next_seq {
            None => 0,
            Some(next) => line.seq.wrapping_sub(next) as i32,
        };
        let pos = stream.next_pos + i64::from(ahead);
        self.line_pos.insert(index, pos);
        let (start, skip) = if ahead < 0 {
            let seen = ahead.unsigned_abs() as usize;
            if seen >= payload.len() {
                return; // all of it was delivered already
            }
            (stream.next_pos, seen)
        } else {
            (pos, 0)
        };
        let delivered = &payload[skip..];
        if ahead > 0 || stream.runs.is_empty() {
            stream.runs.push(Run {
                start,
                bytes: Vec::new(),
                pieces: Vec::new(),
                roots: Vec::new(),
            });
        }
        let run = stream.runs.last_mut().expect("a run was just made");
        run.bytes.extend_from_slice(delivered);
        run.pieces.push(delivered.len());
        stream.next_seq = Some(line.seq.wrapping_add(payload.len() as u32));
        stream.next_pos = start + delivered.len() as i64;
    }

    /// Rewrites every frame of every run in place; bytes outside a frame are blanked.
    fn rewrite(&mut self, people: &mut People, report: &mut Report) -> Result<(), SanitiseError> {
        for key in &self.order {
            let stream = self.streams.get_mut(key).expect("listed streams exist");
            for run in &mut stream.runs {
                let mut assembler = FrameAssembler::new();
                let mut spans = Vec::new();
                let mut at = 0;
                for &piece in &run.pieces {
                    spans.extend(assembler.push_spans(&run.bytes[at..at + piece]));
                    at += piece;
                }
                let mut framed = vec![false; run.bytes.len()];
                run.roots.clear();
                for span in spans {
                    let start = span.start as usize;
                    let frame = &mut run.bytes[start..start + span.len];
                    if let Some(root) = rewrite_frame(frame, span.ty, people, report)? {
                        run.roots.push(root);
                    }
                    framed[start..start + span.len].fill(true);
                }
                for (byte, framed) in run.bytes.iter_mut().zip(framed) {
                    if !framed {
                        *byte = 0;
                    }
                }
            }
        }
        Ok(())
    }

    /// Writes a line's payload as sanitised: each byte the app reads takes its rewritten value,
    /// each byte it never reads (a late packet's) is blanked. Returns the bytes blanked.
    fn fill_payload(&self, index: usize, line: &Line, packet: &mut [u8]) -> usize {
        let pos = self.line_pos[&index];
        let runs = &self.streams[&line.key].runs;
        let mut blanked = 0;
        for (i, byte) in packet[line.payload_at..line.payload_at + line.payload_len]
            .iter_mut()
            .enumerate()
        {
            let at = pos + i as i64;
            let run = runs
                .iter()
                .find(|run| (run.start..run.start + run.bytes.len() as i64).contains(&at));
            *byte = match run {
                Some(run) => run.bytes[(at - run.start) as usize],
                None => {
                    blanked += 1;
                    0
                }
            };
        }
        blanked
    }
}

// --- frames and their protobuf ------------------------------------------------------------------

/// Rewrites one whole frame in place. Returns a history frame's rewritten root.
fn rewrite_frame(
    frame: &mut [u8],
    ty: u16,
    people: &mut People,
    report: &mut Report,
) -> Result<Option<Vec<u8>>, SanitiseError> {
    *report.frames.entry(ty).or_default() += 1;
    if ty == TYPE_KEEPALIVE {
        return Ok(None);
    }
    let body = PREFIX_LEN + inner_header(ty);
    report.bytes_blanked += blank(&mut frame[PREFIX_LEN..body]);
    if ty & COMPRESSED == 0 {
        let root = &mut frame[body..];
        // As `framing` reads them: a plain `0x0003` is history only when it has history lines;
        // otherwise (the player's own line, echoed) it is never read, so all of it is scrubbed.
        let schema = match ty {
            TYPE_OWN_LINE if root.first() == Some(&0x0A) && has_history_lines(root) => {
                Schema::HistoryTop
            }
            TYPE_OWN_LINE => Schema::Other,
            _ => Schema::LiveTop,
        };
        rewrite_message(root, schema, "root", people, report)?;
        return Ok(None);
    }
    let frame_len = frame.len();
    let packed = &mut frame[body..];
    if ty != COMPRESSED | TYPE_OWN_LINE {
        // World state and the like: never read, so nothing of it is kept but the first byte,
        // which tells the framing a zstd frame follows.
        report.bytes_blanked += blank(&mut packed[1..]);
        return Ok(None);
    }
    let Some(mut root) = decompress(packed) else {
        report.bytes_blanked += blank(&mut packed[1..]);
        return Ok(None);
    };
    rewrite_message(&mut root, Schema::HistoryTop, "history", people, report)?;
    let again = compress_to_vec(&root[..], CompressionLevel::Fastest);
    if again.len() > packed.len() {
        return Err(SanitiseError::HistoryGrew {
            frame_len,
            needed: body + again.len(),
        });
    }
    // The decoder reads one zstd frame and ignores what follows it.
    packed[..again.len()].copy_from_slice(&again);
    packed[again.len()..].fill(0);
    report.history_padded += 1;
    Ok(Some(root))
}

fn blank(bytes: &mut [u8]) -> usize {
    bytes.fill(0);
    bytes.len()
}

/// Which message a field belongs to, as `protocol::parser` reads it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Schema {
    /// A live frame's root: `{1: body}`.
    LiveTop,
    /// `{1|2: channel, 2: chat, 4: own line}`; anything else in it is not chat.
    LiveBody,
    /// A history frame's root: `{1: body}`.
    HistoryTop,
    /// `{3: channel, 5: chat, 5: chat, ...}`.
    HistoryBody,
    /// `{1: sequence, 2: sender, 3: time, 4: message}`.
    Chat,
    /// `{1: uid, 2: nickname, 5: level, ...}`.
    Sender,
    /// `{3: text, 7: rich content}`.
    Message,
    /// `{2: chunk, ...}`.
    Rich,
    /// `{1: kind, 2: payload}`.
    Chunk,
    /// `{1: text}` when the chunk is text.
    ChunkPayload,
    /// `{2: channel, 3: text}`.
    OwnLine,
    /// Anything else: strings are filled, messages walked, known uids replaced.
    Other,
}

/// What a field holds, by where it is.
enum Role {
    Message(Schema),
    Text,
    Nickname,
    Uid,
    /// A chat line's sequence or time: never a uid, even when the number is one.
    Keep,
    Unknown,
}

fn role(schema: Schema, tag: u64) -> Role {
    use Schema::*;
    match (schema, tag) {
        (LiveTop, 0x0A) => Role::Message(LiveBody),
        (LiveBody, 0x12) => Role::Message(Chat),
        (LiveBody, 0x22) => Role::Message(OwnLine),
        (HistoryTop, 0x0A) => Role::Message(HistoryBody),
        (HistoryBody, 0x2A) => Role::Message(Chat),
        (Chat, 0x12) => Role::Message(Sender),
        (Chat, 0x22) => Role::Message(Message),
        (Chat, 0x08 | 0x18) => Role::Keep,
        (Sender, 0x08) => Role::Uid,
        (Sender, 0x12) => Role::Nickname,
        (Message, 0x1A) => Role::Text,
        (Message, 0x3A) => Role::Message(Rich),
        (Rich, 0x12) => Role::Message(Chunk),
        (Chunk, 0x12) => Role::Message(ChunkPayload),
        (ChunkPayload, 0x0A) => Role::Text,
        (OwnLine, 0x1A) => Role::Text,
        _ => Role::Unknown,
    }
}

/// Rewrites a protobuf message in place, every replacement the length of what it replaces.
fn rewrite_message(
    buf: &mut [u8],
    schema: Schema,
    path: &str,
    people: &mut People,
    report: &mut Report,
) -> Result<(), SanitiseError> {
    let original = buf.to_vec();
    let base = original.as_ptr() as usize;
    for field in Fields::new(&original) {
        let path = format!("{path}.{}", field.tag);
        let raw_at = field.raw.as_ptr() as usize - base;
        match (role(schema, field.tag), field.value) {
            (Role::Keep, _) | (_, Value::Other) => {}
            (_, Value::Varint(_)) if field.raw.is_empty() => {} // cut short
            (Role::Uid, Value::Varint(uid)) => {
                let new = people.uid(uid, field.raw.len())?;
                write_varint(&mut buf[raw_at..raw_at + field.raw.len()], new)?;
            }
            // Any other varint that is a known uid is replaced, wherever it is (a channel,
            // sequence or time that happened to equal one would fail the self-check).
            (_, Value::Varint(value)) => {
                if let Some(&new) = people.uids.get(&value) {
                    write_varint(&mut buf[raw_at..raw_at + field.raw.len()], new)?;
                } else {
                    people.number_left(&path, value);
                }
            }
            (role, Value::Bytes(bytes)) => {
                let at = bytes.as_ptr() as usize - base;
                let target = &mut buf[at..at + bytes.len()];
                match role {
                    Role::Message(inner) => rewrite_message(target, inner, &path, people, report)?,
                    Role::Text => fill_bytes(target),
                    Role::Nickname => target.copy_from_slice(&people.name(bytes)?),
                    _ => rewrite_unknown(target, &path, people, report)?,
                }
            }
        }
    }
    Ok(())
}

/// A byte field off the known paths: a known name gets its pseudonym, any other string is
/// filled, a message is walked, and what is neither stays (and is listed in the report).
fn rewrite_unknown(
    buf: &mut [u8],
    path: &str,
    people: &mut People,
    report: &mut Report,
) -> Result<(), SanitiseError> {
    if buf.is_empty() {
        return Ok(());
    }
    if let Some(name) = people.names.get(&buf[..]) {
        buf.copy_from_slice(name);
    } else if is_text(buf) {
        fill_bytes(buf);
        report.strings_filled += 1;
    } else if is_message(buf) {
        rewrite_message(buf, Schema::Other, path, people, report)?;
    } else if let Some(varints) = packed_varints(buf) {
        // A packed list (`repeated uint64`): a known uid in it is replaced where it stands.
        for (at, len, value) in varints {
            if let Some(&new) = people.uids.get(&value) {
                write_varint(&mut buf[at..at + len], new)?;
                report.packed_uids += 1;
            } else {
                people.number_left(&format!("{path}[]"), value);
            }
        }
    } else {
        let entry = people.binary_left.entry(path.to_string()).or_default();
        entry.0 += 1;
        entry.1 += buf.len();
    }
    Ok(())
}

/// UTF-8 with no control characters: what a name or a line looks like, and a nested message
/// almost never does (its tags and short lengths are control bytes).
fn is_text(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| !text.chars().any(char::is_control))
}

/// `bytes` as a packed list of varints (where each starts, its length, its value), if they read
/// as varints exactly to their end.
fn packed_varints(bytes: &[u8]) -> Option<Vec<(usize, usize, u64)>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match read_varint(&bytes[at..]) {
            (_, 0) => return None,
            (value, len) => {
                out.push((at, len, value));
                at += len;
            }
        }
    }
    Some(out)
}

/// Whether `bytes` read as protobuf fields exactly to their end.
fn is_message(bytes: &[u8]) -> bool {
    let mut at = 0;
    while at < bytes.len() {
        let Some((tag, read)) = read_tag(&bytes[at..]) else {
            return false;
        };
        if tag >> 3 == 0 {
            return false;
        }
        at += read;
        let rest = &bytes[at..];
        let len = match tag & 7 {
            0 => match read_varint(rest) {
                (_, 0) => return false,
                (_, read) => read,
            },
            1 => 8,
            5 => 4,
            2 => match read_varint(rest) {
                (_, 0) => return false,
                (len, read) => match usize::try_from(len)
                    .ok()
                    .and_then(|len| len.checked_add(read))
                {
                    Some(total) => total,
                    None => return false,
                },
            },
            _ => return false,
        };
        if len > rest.len() {
            return false;
        }
        at += len;
    }
    true
}

/// `value` as a varint of exactly `out.len()` bytes (padded with continuation bits if it is
/// shorter). A uid first met in a padded varint may not fit a shorter one met later.
fn write_varint(out: &mut [u8], value: u64) -> Result<(), SanitiseError> {
    let len = out.len();
    if len < 10 && value >= 1u64 << (7 * len) {
        return Err(SanitiseError::Exhausted { what: "uid", len });
    }
    for (i, byte) in out.iter_mut().enumerate() {
        let bits = (value >> (7 * i)) as u8 & 0x7F;
        *byte = if i + 1 < len { bits | 0x80 } else { bits };
    }
    Ok(())
}

// --- the replacements ---------------------------------------------------------------------------

/// Pseudonyms and uids handed out so far, one per person, in the order they are met.
#[derive(Clone, Default)]
struct People {
    names: HashMap<Vec<u8>, Vec<u8>>,
    uids: HashMap<u64, u64>,
    /// How many of each byte length (names) or varint length (uids) are handed out.
    names_by_len: HashMap<usize, u64>,
    uids_by_len: HashMap<usize, u64>,
    /// Byte fields left as they were, by path: how many and how many bytes.
    binary_left: BTreeMap<String, (usize, usize)>,
    /// Large numbers left as they were, by path: how many, the smallest and the largest.
    numbers_left: BTreeMap<String, (usize, u64, u64)>,
}

const NAME_DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

impl People {
    fn number_left(&mut self, path: &str, value: u64) {
        if value < 10_000 {
            return;
        }
        let entry = self
            .numbers_left
            .entry(path.to_string())
            .or_insert((0, u64::MAX, 0));
        *entry = (entry.0 + 1, entry.1.min(value), entry.2.max(value));
    }

    /// The pseudonym of `name`: `P` and a base-36 number, as many bytes as the name.
    fn name(&mut self, name: &[u8]) -> Result<Vec<u8>, SanitiseError> {
        if let Some(known) = self.names.get(name) {
            return Ok(known.clone());
        }
        let len = name.len();
        let count = self.names_by_len.entry(len).or_default();
        *count += 1;
        let pseudonym = match len {
            0 => Vec::new(),
            1 if *count <= 26 => vec![b'a' + (*count - 1) as u8],
            1 => {
                return Err(SanitiseError::Exhausted {
                    what: "pseudonym",
                    len,
                })
            }
            _ => {
                let mut digits = vec![b'0'; len - 1];
                let mut n = *count;
                for digit in digits.iter_mut().rev() {
                    *digit = NAME_DIGITS[(n % 36) as usize];
                    n /= 36;
                }
                if n > 0 {
                    return Err(SanitiseError::Exhausted {
                        what: "pseudonym",
                        len,
                    });
                }
                [&b"P"[..], &digits].concat()
            }
        };
        self.names.insert(name.to_vec(), pseudonym.clone());
        Ok(pseudonym)
    }

    /// The new uid for `uid`, written in `len` varint bytes: the smallest values of that length,
    /// in the order uids are met. 0 (the player's own line) stays 0.
    fn uid(&mut self, uid: u64, len: usize) -> Result<u64, SanitiseError> {
        if uid == 0 {
            return Ok(0);
        }
        if let Some(&known) = self.uids.get(&uid) {
            return Ok(known);
        }
        let count = self.uids_by_len.entry(len).or_default();
        *count += 1;
        let first = if len <= 1 { 0 } else { 1u64 << (7 * (len - 1)) };
        let new = first + *count;
        let fits = len >= 10 || new < 1u64 << (7 * len);
        if !fits || len == 0 {
            return Err(SanitiseError::Exhausted { what: "uid", len });
        }
        self.uids.insert(uid, new);
        Ok(new)
    }
}

/// Addresses in documentation ranges: servers in 198.51.100.0/24, clients in 192.0.2.0/24.
#[derive(Default)]
struct Addresses {
    map: HashMap<[u8; 4], [u8; 4]>,
    servers: u8,
    clients: u8,
}

impl Addresses {
    fn rewrite(&mut self, packet: &mut [u8]) -> Result<(), SanitiseError> {
        for (at, server) in [(12, true), (16, false)] {
            let mut address = [0u8; 4];
            address.copy_from_slice(&packet[at..at + 4]);
            let new = match self.map.get(&address) {
                Some(&new) => new,
                None => {
                    let (count, prefix) = if server {
                        (&mut self.servers, [198, 51, 100])
                    } else {
                        (&mut self.clients, [192, 0, 2])
                    };
                    if *count == 254 {
                        return Err(SanitiseError::Exhausted {
                            what: "address",
                            len: 4,
                        });
                    }
                    *count += 1;
                    let new = [prefix[0], prefix[1], prefix[2], *count];
                    self.map.insert(address, new);
                    new
                }
            };
            packet[at..at + 4].copy_from_slice(&new);
        }
        Ok(())
    }
}

/// Recomputes the IPv4 header checksum and the TCP checksum of an IPv4/TCP packet.
fn fix_checksums(packet: &mut [u8]) {
    let ihl = usize::from(packet[0] & 0x0F) * 4;
    let total = usize::from(u16::from_be_bytes([packet[2], packet[3]])).min(packet.len());
    packet[10..12].fill(0);
    let ip = checksum(0, &packet[..ihl]);
    packet[10..12].copy_from_slice(&ip.to_be_bytes());

    let tcp_len = total - ihl;
    packet[ihl + 16..ihl + 18].fill(0);
    let mut pseudo = Vec::with_capacity(12);
    pseudo.extend_from_slice(&packet[12..20]);
    pseudo.extend_from_slice(&[0, 6]);
    pseudo.extend_from_slice(&(tcp_len as u16).to_be_bytes());
    let sum = sum_words(sum_words(0, &pseudo), &packet[ihl..total]);
    let tcp = fold(sum);
    packet[ihl + 16..ihl + 18].copy_from_slice(&tcp.to_be_bytes());
}

fn sum_words(mut sum: u32, bytes: &[u8]) -> u32 {
    for pair in bytes.chunks(2) {
        let word = u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    sum
}

fn fold(mut sum: u32) -> u16 {
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn checksum(start: u32, bytes: &[u8]) -> u16 {
    fold(sum_words(start, bytes))
}

// --- the self-check -----------------------------------------------------------------------------

/// Every chat line a capture decodes to, before the duplicate check: what the parser makes of
/// each frame the app would cut, unknown fields included.
fn decode_lines(text: &str) -> Vec<ChatMessage> {
    let mut tracker = StreamTracker::new();
    let mut out = Vec::new();
    for line in &read_lines(text).0 {
        let payload = &line.packet[line.payload_at..line.payload_at + line.payload_len];
        for frame in tracker.process_bytes(line.key, line.seq, payload) {
            let events = match frame.kind {
                FrameKind::Live => parsing_pipeline(&frame.root),
                FrameKind::History => history_pipeline(&frame.root),
            };
            out.extend(events.into_iter().map(|Port5003Event::Chat(chat)| chat));
        }
    }
    out
}

/// How many lines the app would show: what the pipeline emits.
fn shown(text: &str) -> usize {
    let mut pipeline = ChatPipeline::new();
    replay(text)
        .iter()
        .flat_map(|packet| pipeline.feed_network_packet(packet, |_| false, || 1, || {}))
        .filter(|action| matches!(action, PipelineAction::EmitNewMessage(_)))
        .count()
}

/// The sanitised capture must decode to the original's lines, line for line, under the new
/// names, uids and filler, and the app must show as many of them. Returns that count.
fn check_decodes_the_same(
    original: &str,
    was: &[ChatMessage],
    sanitised: &str,
    people: &People,
) -> Result<usize, SanitiseError> {
    let now = decode_lines(sanitised);
    let differ = |detail: String| Err(SanitiseError::DecodesDifferently { detail });
    if was.len() != now.len() {
        return differ(format!("{} lines, not {}", now.len(), was.len()));
    }
    for (i, (was, now)) in was.iter().zip(&now).enumerate() {
        let uid = if was.uid == 0 {
            Some(&0)
        } else {
            people.uids.get(&was.uid)
        };
        let nickname = match people.names.get(was.nickname.as_bytes()) {
            Some(name) => String::from_utf8_lossy(name).into_owned(),
            None if was.nickname == "Me" => "Me".to_string(),
            None => return differ(format!("line {i}: a nickname with no pseudonym")),
        };
        let checks = [
            ("channel", now.channel == was.channel),
            ("uid", Some(&now.uid) == uid),
            ("nickname", now.nickname == nickname),
            ("level", now.level == was.level),
            ("class", now.class_id == was.class_id),
            ("sequence", now.sequence_id == was.sequence_id),
            ("time", now.timestamp == was.timestamp),
            ("text", now.message == fill_text(&was.message)),
            (
                "unknown fields",
                now.unknown_fields.len() == was.unknown_fields.len()
                    && was.unknown_fields.iter().all(|(key, value)| {
                        now.unknown_fields
                            .get(key)
                            .is_some_and(|v| v.len() == value.len())
                    }),
            ),
        ];
        if let Some((field, _)) = checks.iter().find(|(_, same)| !same) {
            let sequence = was.sequence_id;
            return differ(format!(
                "line {i} (sequence {sequence}): its {field} differs"
            ));
        }
    }
    let (was_shown, now_shown) = (shown(original), shown(sanitised));
    if was_shown != now_shown {
        return differ(format!("the app shows {now_shown} lines, not {was_shown}"));
    }
    Ok(was_shown)
}

/// No original nickname (4 bytes or more), uid (as a varint of 4 bytes or more), line (6 bytes
/// or more, unless it is its own filler) or address may be left in the packets, in the streams
/// they make up, or in the history inflated from them. Shorter ones match too much by chance.
fn check_nothing_left(
    was: &[ChatMessage],
    streams: &Streams,
    packets: &[Vec<u8>],
    people: &People,
    addresses: &Addresses,
) -> Result<(), SanitiseError> {
    let mut needles: Vec<(String, Vec<u8>)> = Vec::new();
    for name in people.names.keys().filter(|name| name.len() >= 4) {
        needles.push((format!("a nickname of {} bytes", name.len()), name.clone()));
    }
    for &uid in people.uids.keys() {
        let mut bytes = Vec::new();
        let mut value = uid;
        while value >= 0x80 {
            bytes.push((value as u8 & 0x7F) | 0x80);
            value >>= 7;
        }
        bytes.push(value as u8);
        if bytes.len() >= 4 {
            needles.push(("a uid".to_string(), bytes));
        }
    }
    for chat in was {
        if chat.message.len() >= 6 && fill_text(&chat.message) != chat.message {
            let what = format!("a line of {} bytes", chat.message.len());
            needles.push((what, chat.message.clone().into_bytes()));
        }
    }
    // Every needle is 4 bytes or more: index them by their first four, scan each hay once.
    let mut by_prefix: HashMap<&[u8], Vec<usize>> = HashMap::new();
    for (i, (_, needle)) in needles.iter().enumerate() {
        by_prefix.entry(&needle[..4]).or_default().push(i);
    }
    let mut hays: Vec<(String, &[u8])> = packets
        .iter()
        .enumerate()
        .map(|(i, packet)| (format!("packet {}", i + 1), packet.as_slice()))
        .collect();
    for (s, key) in streams.order.iter().enumerate() {
        for (r, run) in streams.streams[key].runs.iter().enumerate() {
            hays.push((format!("stream {} run {}", s + 1, r + 1), &run.bytes));
            for (h, root) in run.roots.iter().enumerate() {
                let place = format!("stream {} run {} history {}", s + 1, r + 1, h + 1);
                hays.push((place, root));
            }
        }
    }
    for (place, hay) in hays {
        for at in 0..hay.len().saturating_sub(3) {
            for &i in by_prefix.get(&hay[at..at + 4]).into_iter().flatten() {
                let (what, needle) = &needles[i];
                if hay[at..].starts_with(needle) {
                    return Err(SanitiseError::Leaks {
                        detail: format!("{what}, in {place} at byte {at}"),
                    });
                }
            }
        }
    }
    for packet in packets {
        for at in [12, 16] {
            let address: [u8; 4] = packet[at..at + 4].try_into().expect("four bytes");
            if addresses.map.contains_key(&address) {
                return Err(SanitiseError::Leaks {
                    detail: "an address".to_string(),
                });
            }
        }
    }
    Ok(())
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
        // A live frame that is not chat: a record `{1: uid}` in field 5 and, deeper, a packed
        // list of uids (both seen in a real capture).
        let alice_uid = varint(37_276_266);
        let record = [&[0x08][..], &alice_uid].concat();
        let packed = [&[0x0A, alice_uid.len() as u8][..], &alice_uid].concat();
        let deeper = [&[0x22, packed.len() as u8][..], &packed].concat();
        let inner = [&[0x10, 0x01, 0x42, deeper.len() as u8][..], &deeper].concat();
        let body = [
            &[0x0A, inner.len() as u8][..],
            &inner,
            &[0x2A, record.len() as u8],
            &record,
        ]
        .concat();
        let not_chat = frame(0x0002, 16, &[&[0x0A, body.len() as u8][..], &body].concat());
        let stream_a = [
            live_alice.clone(),
            keepalive,
            world,
            history,
            echo,
            own,
            not_chat,
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
    fn the_self_check_refuses_a_capture_that_decodes_differently() {
        let original = capture();
        let out = sanitise(&original).unwrap();
        let was = decode_lines(&original);
        // People as sanitise built them; the original itself is not their sanitised form.
        let (lines, _) = read_lines(&original);
        let mut streams = Streams::default();
        for (index, line) in lines.iter().enumerate() {
            streams.deliver(index, line);
        }
        let mut people = People::default();
        streams
            .rewrite(&mut people, &mut Report::default())
            .unwrap();
        assert!(check_decodes_the_same(&original, &was, &out.text, &people).is_ok());
        assert!(matches!(
            check_decodes_the_same(&original, &was, &original, &people),
            Err(SanitiseError::DecodesDifferently { .. })
        ));
        // A line lost is refused too.
        let fewer: String = out
            .text
            .lines()
            .take(2)
            .map(|l| {
                format!(
                    "{l}
"
                )
            })
            .collect();
        assert!(matches!(
            check_decodes_the_same(&original, &was, &fewer, &people),
            Err(SanitiseError::DecodesDifferently { .. })
        ));
    }

    #[test]
    fn the_leak_check_refuses_what_is_left_of_the_original() {
        let original = capture();
        let (lines, _) = read_lines(&original);
        let mut streams = Streams::default();
        for (index, line) in lines.iter().enumerate() {
            streams.deliver(index, line);
        }
        let mut people = People::default();
        let untouched = streams.clone();
        streams
            .rewrite(&mut people, &mut Report::default())
            .unwrap();
        let was = decode_lines(&original);
        let raw: Vec<Vec<u8>> = lines.iter().map(|line| line.packet.clone()).collect();
        let mut addresses = Addresses::default();
        for packet in &raw {
            addresses.rewrite(&mut packet.clone()).unwrap();
        }
        // The original packets, and the original streams: both still hold names.
        assert!(matches!(
            check_nothing_left(&was, &streams, &raw, &people, &addresses),
            Err(SanitiseError::Leaks { .. })
        ));
        let blank: Vec<Vec<u8>> = raw.iter().map(|p| vec![0; p.len()]).collect();
        assert!(matches!(
            check_nothing_left(&was, &untouched, &blank, &people, &addresses),
            Err(SanitiseError::Leaks { .. })
        ));
    }

    #[test]
    fn numbers_it_cannot_place_are_listed_for_review() {
        // The echoed own line is never read, so its sequence and time are walked as unknown
        // numbers; only the time is large enough to list.
        let out = sanitise(&capture()).unwrap();
        assert_eq!(
            out.report.numbers_left,
            ["root.10.18.24: 1 numbers, 1759420004 to 1759420004"]
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
