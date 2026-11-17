//! The server's verdict on a send: the [`Verdict`] a Return means, and a [`VerdictMatcher`] that
//! pairs each send's `call_id` with its Return within [`VERDICT_WAIT`].
//!
//! Pure: no clock inside (every call gets `now`), no wiring. The Return arrives without a pid or
//! target, so the caller gives the key a send was registered under.

use crate::protocol::rpc_return::RpcReturn;
use resonance_types::SendChatChannel;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a send waits for its Return before it ends as [`Verdict::NotConfirmed`].
pub const VERDICT_WAIT: Duration = Duration::from_millis(1000);
/// Bound of each of the matcher's two maps.
pub const MAX_TRACKED: usize = 64;
/// A cooldown longer than this is taken for a wrong clock.
pub const MAX_COOLDOWN: Duration = Duration::from_secs(600);

/// The client a send belongs to: the `send_chat` target as given (`ClientSpacing`'s key).
pub type ClientKey = Option<u32>;

/// What the server said about one send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Sent,
    /// Refused for the cooldown, its end unknown.
    TooFrequent,
    /// Refused for the cooldown, whole seconds left (rounded up, 1..=600).
    Cooldown {
        remaining: Duration,
    },
    /// No Return within [`VERDICT_WAIT`]: sent, not confirmed.
    NotConfirmed,
    Other(u32),
}

/// A send that has its verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub client: ClientKey,
    pub call_id: u32,
    pub channel: SendChatChannel,
    pub verdict: Verdict,
    /// The exact time until the server's `cd_end_time`, for the cooldown gate.
    pub cooldown: Option<Duration>,
}

/// The verdict of a Return. `fallback_server_ms` is the server's now (Unix ms) when the Return
/// carries none of its own.
pub fn verdict(_ret: &RpcReturn, _fallback_server_ms: Option<u64>) -> Verdict {
    Verdict::NotConfirmed
}

/// The exact time until the Return's `cd_end_time`; `Some` only for a plausible future end.
pub fn cooldown_remaining(_ret: &RpcReturn, _fallback_server_ms: Option<u64>) -> Option<Duration> {
    None
}

/// Pairs sends with their Returns.
#[derive(Debug, Default)]
pub struct VerdictMatcher {
    pending: HashMap<(ClientKey, u32), (SendChatChannel, Instant)>,
    early: HashMap<(ClientKey, u32), (RpcReturn, Option<u64>, Instant)>,
}

impl VerdictMatcher {
    pub fn register(
        &mut self,
        _client: ClientKey,
        _call_id: u32,
        _channel: SendChatChannel,
        _sent_at: Instant,
    ) -> Vec<Resolved> {
        Vec::new()
    }

    pub fn on_return(
        &mut self,
        _client: ClientKey,
        _ret: &RpcReturn,
        _fallback_server_ms: Option<u64>,
        _now: Instant,
    ) -> Option<Resolved> {
        None
    }

    pub fn poll(&mut self, _now: Instant) -> Vec<Resolved> {
        Vec::new()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn early_len(&self) -> usize {
        self.early.len()
    }
}
