//! Raw capture of the game's port-5003 traffic, for reverse-engineering the
//! protocol (W2). One packet per line, appended as it arrives:
//!
//! ```text
//! <unix_ms>\t<hex of the whole IPv4 packet>
//! ```
//!
//! The whole IP packet is kept, not just the payload: the TCP sequence numbers
//! are what tells a retransmit from new data, and a line replays straight
//! through [`ChatPipeline::feed_network_packet`](super::ChatPipeline).

use etherparse::{PacketHeaders, TransportHeader};
use std::io::{self, Write};

/// A capture file stops growing here unless told otherwise.
pub const DEFAULT_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Is this packet worth recording: server → client TCP data on port 5003?
/// The raw socket sees every packet on the interface; the rest (and the empty
/// ACKs) would only bury the chat.
pub fn is_capturable(packet: &[u8]) -> bool {
    let Ok(headers) = PacketHeaders::from_ip_slice(packet) else {
        return false;
    };
    let Some(TransportHeader::Tcp(tcp)) = headers.transport else {
        return false;
    };
    tcp.source_port == 5003 && !headers.payload.slice().is_empty()
}

/// One capture line, without the trailing newline.
pub fn encode_line(ts_ms: u64, packet: &[u8]) -> String {
    let mut line = format!("{ts_ms}\t");
    for byte in packet {
        line.push_str(&format!("{byte:02x}"));
    }
    line
}

/// The inverse of [`encode_line`]; `None` for anything that is not a capture line.
pub fn decode_line(line: &str) -> Option<(u64, Vec<u8>)> {
    let (ts, hex) = line.trim().split_once('\t')?;
    let ts_ms = ts.parse().ok()?;
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    let packet = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    Some((ts_ms, packet))
}

/// The packets of a capture file, in order. Blank lines, `#` comments and
/// malformed lines and packets cut short (a half-written last line after a
/// crash) are skipped.
pub fn replay(text: &str) -> Vec<Vec<u8>> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(decode_line)
        .map(|(_, packet)| packet)
        .filter(|packet| is_capturable(packet))
        .collect()
}

/// What [`RawCaptureWriter::record`] did with a packet.
#[derive(Debug, PartialEq, Eq)]
pub enum Recorded {
    Written,
    /// Not port-5003 data; nothing written.
    Skipped,
    /// The size cap is reached; nothing written, and nothing will be.
    Full,
}

/// Appends capture lines to `out` until `max_bytes` is reached.
pub struct RawCaptureWriter<W: Write> {
    out: W,
    written: u64,
    max_bytes: u64,
}

impl<W: Write> RawCaptureWriter<W> {
    pub fn new(out: W, max_bytes: u64) -> Self {
        Self {
            out,
            written: 0,
            max_bytes,
        }
    }

    pub fn record(&mut self, ts_ms: u64, packet: &[u8]) -> io::Result<Recorded> {
        if !is_capturable(packet) {
            return Ok(Recorded::Skipped);
        }
        let mut line = encode_line(ts_ms, packet);
        line.push('\n');
        if self.written + line.len() as u64 > self.max_bytes {
            return Ok(Recorded::Full);
        }
        self.out.write_all(line.as_bytes())?;
        self.written += line.len() as u64;
        Ok(Recorded::Written)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    pub fn bytes_written(&self) -> u64 {
        self.written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use etherparse::PacketBuilder;

    fn packet(src_port: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PacketBuilder::ipv4([10, 0, 0, 1], [10, 0, 0, 2], 64)
            .tcp(src_port, 40000, 7, 0)
            .write(&mut out, payload)
            .unwrap();
        out
    }

    #[test]
    fn a_line_round_trips() {
        let pkt = packet(5003, b"hello");
        let line = encode_line(1_700_000_000_123, &pkt);
        assert!(!line.contains('\n'));
        assert_eq!(decode_line(&line), Some((1_700_000_000_123, pkt)));
    }

    #[test]
    fn a_line_is_timestamp_tab_hex() {
        assert_eq!(encode_line(5, &[0x00, 0xab, 0xff]), "5\t00abff");
    }

    #[test]
    fn malformed_lines_do_not_decode() {
        for bad in ["", "no tab", "x\t00", "5\t", "5\t0", "5\tzz", "5\t00é"] {
            assert_eq!(decode_line(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_data_from_port_5003_is_capturable() {
        assert!(is_capturable(&packet(5003, b"chat")));
        assert!(!is_capturable(&packet(5003, b""))); // bare ACK
        assert!(!is_capturable(&packet(443, b"tls")));
        assert!(!is_capturable(b"not an ip packet"));
    }

    #[test]
    fn the_writer_appends_lines_and_skips_other_traffic() {
        let mut w = RawCaptureWriter::new(Vec::new(), DEFAULT_MAX_BYTES);
        let a = packet(5003, b"one");
        let b = packet(5003, b"two");
        assert_eq!(w.record(1, &a).unwrap(), Recorded::Written);
        assert_eq!(w.record(2, &packet(443, b"x")).unwrap(), Recorded::Skipped);
        assert_eq!(w.record(3, &b).unwrap(), Recorded::Written);
        let text = String::from_utf8(w.out.clone()).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(replay(&text), vec![a, b]);
        assert_eq!(w.bytes_written(), text.len() as u64);
    }

    #[test]
    fn the_writer_stops_at_the_size_cap_without_a_partial_line() {
        let pkt = packet(5003, b"one");
        let line_len = encode_line(1, &pkt).len() as u64 + 1;
        let mut w = RawCaptureWriter::new(Vec::new(), line_len * 2);
        assert_eq!(w.record(1, &pkt).unwrap(), Recorded::Written);
        assert_eq!(w.record(2, &pkt).unwrap(), Recorded::Written);
        assert_eq!(w.record(3, &pkt).unwrap(), Recorded::Full);
        assert_eq!(w.record(4, &pkt).unwrap(), Recorded::Full);
        assert_eq!(w.out.len() as u64, line_len * 2);
        assert!(w.out.ends_with(b"\n"));
    }

    #[test]
    fn replay_skips_comments_blanks_and_a_torn_last_line() {
        let pkt = packet(5003, b"one");
        let text = format!("# note\n\n{}\n5\t00ab", encode_line(1, &pkt));
        assert_eq!(replay(&text), vec![pkt]);
    }

    #[test]
    fn a_replayed_capture_feeds_the_pipeline() {
        use crate::capture::{ChatPipeline, PipelineAction};
        // A truncated/garbled payload must not emit chat or panic.
        let mut w = RawCaptureWriter::new(Vec::new(), DEFAULT_MAX_BYTES);
        w.record(1, &packet(5003, b"\x00\x00\x00\x05junk!"))
            .unwrap();
        let text = String::from_utf8(w.out).unwrap();
        let mut pipeline = ChatPipeline::new();
        for pkt in replay(&text) {
            let actions = pipeline.feed_network_packet(&pkt, |_| false, || 1, || {});
            assert!(actions
                .iter()
                .all(|a| !matches!(a, PipelineAction::EmitNewMessage(_, _))));
        }
    }
}
