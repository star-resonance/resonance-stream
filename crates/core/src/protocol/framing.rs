//! Splits a port-5003 TCP stream into frames.
//!
//! Every frame is `[u32 total length, big endian, header included][u16 type][body]`
//! (read off a real capture; the open questions are in `.memory/roadmap/core-review-2026-09-30.md`):
//!
//! | type     | body                                               | used |
//! |----------|----------------------------------------------------|------|
//! | `0x0002` | 16-byte header, then a protobuf root: live chat    | yes  |
//! | `0x0003` | 12-byte header, then a root: the player's own line, echoed (always also sent as `0x0002`) -- or, when the root has field 5, a one-line channel history | history only |
//! | `0x0004` | none: keepalive, 6 bytes                            | no   |
//! | `0x8002` | 16-byte header, zstd: world state, not chat         | no   |
//! | `0x8003` | 12-byte header, zstd, then a root: channel history  | yes  |
//!
//! A protobuf root starts with `0x0A`. Bit 15 of the type means the body after
//! the header is a zstd frame.
//!
//! The sniffer can join a connection mid-frame, and a lost segment leaves the
//! stream cut. The assembler is then *unsynced* and looks for the first spot
//! where a run of plausible frame headers begins; it never trusts a length it
//! has not checked.

use crate::protocol::compression::decompress;
use crate::protocol::decoder::{field_end, read_varint, Fields, Value};

/// A frame longer than this is not a frame (the biggest seen is ~4 KB).
pub const MAX_FRAME_LEN: usize = 1 << 20;

/// `[len][type]`.
pub(crate) const PREFIX_LEN: usize = 6;
pub(crate) const COMPRESSED: u16 = 0x8000;
pub(crate) const TYPE_CHAT: u16 = 0x0002;
pub(crate) const TYPE_OWN_LINE: u16 = 0x0003;
pub(crate) const TYPE_KEEPALIVE: u16 = 0x0004;
const ROOT_TAG: u8 = 0x0A;
const ZSTD_FIRST_BYTE: u8 = 0x28;

/// What a frame's root holds; the parser reads each differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// `0x0002`: one chat line, with its channel.
    Live,
    /// `0x8003`, or a plain `0x0003` whose root has field 5: the channel's
    /// recent lines, newest first.
    History,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameKind,
    /// A protobuf root (starts with `0x0A`), decompressed if it came compressed.
    pub root: Vec<u8>,
}

/// Where a complete frame sits in the stream: its first byte, counted from the first byte the
/// assembler was given, its length (header included) and its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSpan {
    pub start: u64,
    pub len: usize,
    pub ty: u16,
}

#[derive(Debug, Default)]
pub struct FrameAssembler {
    buf: Vec<u8>,
    synced: bool,
    /// Bytes given to the assembler before `buf[0]`.
    dropped: u64,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds the next in-order TCP payload; returns every frame it completes.
    pub fn push(&mut self, segment: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        self.cut(segment, |span, frame| {
            frames.extend(decode(span.ty, &frame[PREFIX_LEN..]));
        });
        frames
    }

    /// [`push`](Self::push) as positions: where each frame the segment completes sits, whether
    /// or not it decodes. The capture sanitiser rewrites those bytes in place.
    pub fn push_spans(&mut self, segment: &[u8]) -> Vec<FrameSpan> {
        let mut spans = Vec::new();
        self.cut(segment, |span, _| spans.push(span));
        spans
    }

    /// Appends `segment` and hands every frame it completes, whole (`[len][type][body]`), to
    /// `take` with where it sits, in stream order.
    fn cut(&mut self, segment: &[u8], mut take: impl FnMut(FrameSpan, &[u8])) {
        self.buf.extend_from_slice(segment);
        loop {
            if !self.synced {
                match find_sync(&self.buf) {
                    Some(start) => {
                        self.drop_front(start);
                        self.synced = true;
                    }
                    None => {
                        // Nothing to trust: keep only what could be the start
                        // of a header the next segment completes.
                        let keep = self.buf.len().min(PREFIX_LEN - 1);
                        self.drop_front(self.buf.len() - keep);
                        return;
                    }
                }
            }
            if self.buf.len() < PREFIX_LEN {
                return;
            }
            let Some(len) = header_len(&self.buf) else {
                // Not a frame after all: look for the next real one.
                self.synced = false;
                self.drop_front(1);
                continue;
            };
            if self.buf.len() < len {
                return;
            }
            let ty = u16::from_be_bytes([self.buf[4], self.buf[5]]);
            let span = FrameSpan {
                start: self.dropped,
                len,
                ty,
            };
            take(span, &self.buf[..len]);
            self.drop_front(len);
        }
    }

    fn drop_front(&mut self, n: usize) {
        self.buf.drain(..n);
        self.dropped += n as u64;
    }

    /// Forgets the partial frame: the stream is cut (a segment went missing).
    pub fn reset(&mut self) {
        let n = self.buf.len();
        self.drop_front(n);
        self.synced = false;
    }

    /// Bytes waiting for the rest of a frame.
    pub fn pending_len(&self) -> usize {
        self.buf.len()
    }
}

/// The length of the frame starting at `data[0]` if its header is plausible:
/// a sane length, a type this protocol has, and -- when the bytes are already
/// there -- what the type says comes first in the body.
fn header_len(data: &[u8]) -> Option<usize> {
    let len = usize::try_from(u32::from_be_bytes(data.get(..4)?.try_into().ok()?)).ok()?;
    let ty = u16::from_be_bytes(data.get(4..PREFIX_LEN)?.try_into().ok()?);
    if !(PREFIX_LEN..=MAX_FRAME_LEN).contains(&len) {
        return None;
    }
    match ty & !COMPRESSED {
        TYPE_KEEPALIVE => (ty == TYPE_KEEPALIVE && len == PREFIX_LEN).then_some(len),
        TYPE_CHAT | TYPE_OWN_LINE => {
            let body_start = PREFIX_LEN + inner_header(ty);
            if len <= body_start {
                return None;
            }
            let first = if ty & COMPRESSED != 0 {
                ZSTD_FIRST_BYTE
            } else {
                ROOT_TAG
            };
            match data.get(body_start) {
                Some(&byte) if byte != first => None,
                _ => Some(len),
            }
        }
        _ => None,
    }
}

/// Bytes between the type and the root (or zstd frame).
pub(crate) fn inner_header(ty: u16) -> usize {
    match ty & !COMPRESSED {
        TYPE_CHAT => 16,
        _ => 12,
    }
}

/// A length that could be a frame's, whatever its type.
fn sane_len(data: &[u8]) -> Option<usize> {
    let len = usize::try_from(u32::from_be_bytes(data.get(..4)?.try_into().ok()?)).ok()?;
    (PREFIX_LEN..=MAX_FRAME_LEN).contains(&len).then_some(len)
}

/// The first offset in `data` where a frame this protocol has starts, and the
/// frames after it (of any type) chain by sane lengths up to the end of the
/// data; the last one may be unfinished.
fn find_sync(data: &[u8]) -> Option<usize> {
    (0..data.len().saturating_sub(PREFIX_LEN - 1)).find(|&start| {
        let Some(first) = header_len(&data[start..]) else {
            return false;
        };
        let mut at = start + first;
        while data.len() - at.min(data.len()) >= PREFIX_LEN {
            match sane_len(&data[at..]) {
                Some(len) => at += len,
                None => return false,
            }
        }
        true
    })
}

/// A frame's body (what follows `[len][type]`) as the parser gets it, if it is chat or history.
pub(crate) fn decode(ty: u16, body: &[u8]) -> Option<Frame> {
    let payload = body.get(inner_header(ty)..)?;
    let root = if ty & COMPRESSED != 0 {
        decompress(payload)?
    } else {
        payload.to_vec()
    };
    if root.first() != Some(&ROOT_TAG) {
        return None;
    }
    let kind = match ty {
        TYPE_CHAT => FrameKind::Live,
        t if t == COMPRESSED | TYPE_OWN_LINE => FrameKind::History,
        // Plain `0x0003`: a one-line history has `{3: channel, 5: chat}`, the
        // echo of the player's own line does not (and comes again as `0x0002`).
        TYPE_OWN_LINE if has_history_lines(&root) => FrameKind::History,
        _ => return None,
    };
    Some(Frame { kind, root })
}

/// Whether a root carries chat lines in field 5 (the history layout).
fn has_history_lines(root: &[u8]) -> bool {
    let (len, read) = read_varint(&root[1..]);
    let start = 1 + read;
    let end = field_end(start, len, root.len());
    Fields::new(&root[start..end]).any(|f| f.number() == 5 && matches!(f.value, Value::Bytes(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};

    fn root(body: &[u8]) -> Vec<u8> {
        let mut r = vec![ROOT_TAG, body.len() as u8];
        r.extend_from_slice(body);
        r
    }

    /// `[len][type][inner header][payload]`
    fn frame_of(ty: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 4];
        out.extend(ty.to_be_bytes());
        out.extend(vec![0xEE; inner_header(ty)]);
        out.extend_from_slice(payload);
        let len = out.len() as u32;
        out[..4].copy_from_slice(&len.to_be_bytes());
        out
    }

    fn live(body: &[u8]) -> Vec<u8> {
        frame_of(TYPE_CHAT, &root(body))
    }

    fn history(body: &[u8]) -> Vec<u8> {
        let packed = compress_to_vec(&root(body)[..], CompressionLevel::Fastest);
        frame_of(COMPRESSED | TYPE_OWN_LINE, &packed)
    }

    fn keepalive() -> Vec<u8> {
        vec![0, 0, 0, 6, 0, 4]
    }

    fn roots(frames: &[Frame]) -> Vec<&[u8]> {
        frames.iter().map(|f| f.root.as_slice()).collect()
    }

    #[test]
    fn a_live_frame_yields_its_root() {
        let got = FrameAssembler::new().push(&live(b"hello"));
        assert_eq!(
            got,
            vec![Frame {
                kind: FrameKind::Live,
                root: root(b"hello")
            }]
        );
    }

    #[test]
    fn a_history_frame_is_inflated() {
        let got = FrameAssembler::new().push(&history(b"backlog"));
        assert_eq!(
            got,
            vec![Frame {
                kind: FrameKind::History,
                root: root(b"backlog")
            }]
        );
    }

    #[test]
    fn frames_that_are_not_chat_are_skipped_by_their_length() {
        let stream = [
            keepalive(),
            frame_of(TYPE_OWN_LINE, &root(b"echo")),
            frame_of(
                COMPRESSED | TYPE_CHAT,
                &compress_to_vec(&root(b"state")[..], CompressionLevel::Fastest),
            ),
            live(b"one"),
            keepalive(),
        ]
        .concat();
        let got = FrameAssembler::new().push(&stream);
        assert_eq!(roots(&got), [&root(b"one")[..]]);
    }

    /// Root `{ 3: channel, 5: chat }` as the server sends a one-line history.
    fn one_line_history_root() -> Vec<u8> {
        let chat = [&[0x08, 0x01][..], b"\x12\x02hi"].concat();
        let mut body = vec![0x18, 0x09, 0x2A, chat.len() as u8];
        body.extend(chat);
        root(&body)
    }

    #[test]
    fn a_plain_0x0003_frame_with_history_lines_is_history() {
        let got = FrameAssembler::new().push(&frame_of(TYPE_OWN_LINE, &one_line_history_root()));
        assert_eq!(
            got,
            vec![Frame {
                kind: FrameKind::History,
                root: one_line_history_root()
            }]
        );
    }

    #[test]
    fn a_plain_0x0003_echo_stays_skipped() {
        // `{ 3: chat, 4: server time }`: no field 5, so not history.
        let echo = root(&[0x1A, 0x02, 0x08, 0x01, 0x20, 0x05]);
        let got = FrameAssembler::new().push(&frame_of(TYPE_OWN_LINE, &echo));
        assert!(got.is_empty());
    }

    #[test]
    fn coalesced_frames_all_come_out_in_order() {
        let stream = [live(b"one"), history(b"two"), live(b"three")].concat();
        let got = FrameAssembler::new().push(&stream);
        assert_eq!(
            roots(&got),
            [&root(b"one")[..], &root(b"two")[..], &root(b"three")[..]]
        );
        assert_eq!(got[1].kind, FrameKind::History);
    }

    #[test]
    fn spans_say_where_each_frame_sits_counting_dropped_bytes() {
        let junk = [0xFFu8, 0xFF, 0xFF];
        let (a, k, b) = (live(b"first"), keepalive(), history(b"second"));
        let whole = [&junk[..], &a, &k, &b, &[0, 0]].concat();
        let cut = junk.len() + a.len() + 3; // inside the keepalive
        let mut fa = FrameAssembler::new();
        let mut spans = fa.push_spans(&whole[..cut]);
        spans.extend(fa.push_spans(&whole[cut..]));
        let at = |offset: usize, frame: &[u8]| FrameSpan {
            start: offset as u64,
            len: frame.len(),
            ty: u16::from_be_bytes([frame[4], frame[5]]),
        };
        let (sa, sk) = (junk.len(), junk.len() + a.len());
        assert_eq!(spans, [at(sa, &a), at(sk, &k), at(sk + k.len(), &b)]);
        // After a reset the count goes on from every byte given so far.
        fa.reset();
        let again = fa.push_spans(&a);
        assert_eq!(again, [at(whole.len(), &a)]);
    }

    #[test]
    fn a_frame_cut_anywhere_is_joined() {
        let whole = [live(b"before"), history(b"cut me up"), live(b"after")].concat();
        for cut in 1..whole.len() {
            let mut fa = FrameAssembler::new();
            let mut got = fa.push(&whole[..cut]);
            got.extend(fa.push(&whole[cut..]));
            assert_eq!(
                roots(&got),
                [
                    &root(b"before")[..],
                    &root(b"cut me up")[..],
                    &root(b"after")[..]
                ],
                "cut at {cut}"
            );
            assert_eq!(fa.pending_len(), 0);
        }
    }

    #[test]
    fn a_frame_one_byte_at_a_time_is_joined() {
        let mut fa = FrameAssembler::new();
        let mut got = Vec::new();
        for byte in [live(b"slow"), history(b"slower")].concat() {
            got.extend(fa.push(&[byte]));
        }
        assert_eq!(roots(&got), [&root(b"slow")[..], &root(b"slower")[..]]);
    }

    #[test]
    fn joining_mid_frame_finds_the_next_frame() {
        // The sniffer started in the middle of a frame: its tail comes first.
        let tail_of_cut_frame = &live(b"cut off at the start")[9..];
        let mut fa = FrameAssembler::new();
        let mut got = fa.push(tail_of_cut_frame);
        got.extend(fa.push(&[live(b"one"), live(b"two")].concat()));
        assert_eq!(roots(&got), [&root(b"one")[..], &root(b"two")[..]]);
    }

    #[test]
    fn garbage_in_front_does_not_hide_the_next_frame() {
        let mut fa = FrameAssembler::new();
        assert!(fa.push(&[0x0A, 0xFF, 0x13, 0x37]).is_empty());
        let got = fa.push(&live(b"clean"));
        assert_eq!(roots(&got), [&root(b"clean")[..]]);
    }

    #[test]
    fn a_frame_after_an_unknown_type_is_still_found() {
        let unknown = vec![0, 0, 0, 10, 0x12, 0x34, 1, 2, 3, 4];
        let got = FrameAssembler::new().push(&[live(b"one"), unknown, live(b"two")].concat());
        assert_eq!(roots(&got), [&root(b"one")[..], &root(b"two")[..]]);
    }

    #[test]
    fn a_body_that_will_not_inflate_costs_its_frame_only() {
        let bad = frame_of(COMPRESSED | TYPE_OWN_LINE, &[ZSTD_FIRST_BYTE, 1, 2, 3, 4]);
        let got = FrameAssembler::new().push(&[bad, live(b"next")].concat());
        assert_eq!(roots(&got), [&root(b"next")[..]]);
    }

    #[test]
    fn a_length_that_cannot_be_true_resyncs() {
        let mut bad = live(b"liar");
        bad[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        let got = FrameAssembler::new().push(&[bad, live(b"honest")].concat());
        assert_eq!(roots(&got), [&root(b"honest")[..]]);
    }

    #[test]
    fn reset_forgets_the_partial_frame() {
        let mut fa = FrameAssembler::new();
        let whole = live(b"half");
        assert!(fa.push(&whole[..8]).is_empty());
        assert_eq!(fa.pending_len(), 8);
        fa.reset();
        assert_eq!(fa.pending_len(), 0);
        assert_eq!(roots(&fa.push(&live(b"fresh"))), [&root(b"fresh")[..]]);
    }

    #[test]
    fn pending_stays_bounded_on_endless_garbage() {
        let mut fa = FrameAssembler::new();
        for i in 0..1000u32 {
            let junk: Vec<u8> = (0..1400u32)
                .map(|b| (b.wrapping_mul(31) ^ i) as u8)
                .collect();
            fa.push(&junk);
            assert!(
                fa.pending_len() <= MAX_FRAME_LEN + 1400,
                "{}",
                fa.pending_len()
            );
        }
    }

    #[test]
    fn a_huge_declared_length_waits_at_most_a_frame() {
        // Plausible header, absurd (but allowed) length, then junk: never more
        // than MAX_FRAME_LEN is held.
        let mut head = vec![0x00, 0x0F, 0xFF, 0xFF, 0x00, 0x02];
        head.extend(vec![0xEE; 16]);
        head.push(ROOT_TAG);
        let mut fa = FrameAssembler::new();
        fa.push(&head);
        assert!(fa.pending_len() <= MAX_FRAME_LEN);
    }
}
