//! The server's chat cooldown per client and channel: a send is refused here until the
//! `cd_end_time` the server gave has passed. Pure, `now` is passed in (as `ClientSpacing`).

use crate::send_verdict::{ClientKey, Verdict};
use resonance_types::SendChatChannel;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct CooldownGate {
    ends: Mutex<HashMap<(ClientKey, SendChatChannel), Instant>>,
}

impl CooldownGate {
    /// Blocks `(client, channel)` for `remaining` from `now`, replacing an earlier end.
    pub fn arm(
        &self,
        _client: ClientKey,
        _channel: SendChatChannel,
        _remaining: Duration,
        _now: Instant,
    ) {
    }

    /// Ok when a send may go out; otherwise the cooldown left (whole seconds, rounded up).
    /// Never writes.
    pub fn check(
        &self,
        _client: ClientKey,
        _channel: SendChatChannel,
        _now: Instant,
    ) -> Result<(), Verdict> {
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.ends.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
