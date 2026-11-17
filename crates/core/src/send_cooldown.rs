//! The server's chat cooldown per client and channel: a send is refused here until the
//! `cd_end_time` the server gave has passed. Pure, `now` is passed in (as `ClientSpacing`).

use crate::send_verdict::{ClientKey, Verdict};
use resonance_types::SendChatChannel;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct CooldownGate {
    ends: Mutex<HashMap<(ClientKey, SendChatChannel), Instant>>,
}

impl CooldownGate {
    /// Blocks `(client, channel)` for `remaining` from `now`, replacing an earlier end (longer or
    /// shorter: the server's latest word wins). Keys already ended at `now` are dropped.
    pub fn arm(
        &self,
        client: ClientKey,
        channel: SendChatChannel,
        remaining: Duration,
        now: Instant,
    ) {
        let mut ends = self.ends.lock().unwrap_or_else(PoisonError::into_inner);
        ends.retain(|_, end| now < *end);
        ends.insert((client, channel), now + remaining);
    }

    /// Ok when a send may go out; otherwise the cooldown left (whole seconds, rounded up, at
    /// least 1). Never writes.
    pub fn check(
        &self,
        client: ClientKey,
        channel: SendChatChannel,
        now: Instant,
    ) -> Result<(), Verdict> {
        let ends = self.ends.lock().unwrap_or_else(PoisonError::into_inner);
        match ends.get(&(client, channel)) {
            Some(end) if now < *end => Err(Verdict::Cooldown {
                remaining: Duration::from_secs(
                    (*end - now).as_millis().div_ceil(1000).max(1) as u64
                ),
            }),
            _ => Ok(()),
        }
    }

    /// How many keys are kept.
    pub fn len(&self) -> usize {
        self.ends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rpc_return::RpcReturn;
    use crate::send_verdict::VerdictMatcher;

    const W: SendChatChannel = SendChatChannel::World;
    const A: ClientKey = Some(7);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }
    fn cooldown(n: u64) -> Verdict {
        Verdict::Cooldown { remaining: secs(n) }
    }

    #[test]
    fn a_new_gate_allows() {
        let gate = CooldownGate::default();
        assert_eq!(gate.check(A, W, Instant::now()), Ok(()));
        assert!(gate.is_empty());
    }

    #[test]
    fn an_armed_gate_refuses_until_the_end_with_whole_seconds_rounded_up() {
        let gate = CooldownGate::default();
        let t0 = Instant::now();
        gate.arm(A, W, secs(5), t0);
        assert_eq!(gate.check(A, W, t0 + ms(1200)), Err(cooldown(4)));
        assert_eq!(gate.check(A, W, t0 + ms(4999)), Err(cooldown(1)));
        assert_eq!(gate.check(A, W, t0 + ms(5000)), Ok(()));
        assert_eq!(gate.check(A, W, t0), Err(cooldown(5)));
    }

    #[test]
    fn only_its_client_and_channel_are_blocked() {
        let gate = CooldownGate::default();
        let t0 = Instant::now();
        gate.arm(A, W, secs(5), t0);
        let at = t0 + secs(1);
        assert_eq!(gate.check(Some(42), W, at), Ok(()));
        assert_eq!(gate.check(A, SendChatChannel::Party, at), Ok(()));
        assert_eq!(gate.check(A, SendChatChannel::Say, at), Ok(()));
        assert_eq!(gate.check(None, W, at), Ok(()));
    }

    #[test]
    fn a_new_arm_replaces_the_old_end_shorter_too() {
        let gate = CooldownGate::default();
        let t0 = Instant::now();
        gate.arm(A, W, secs(5), t0);
        gate.arm(A, W, secs(2), t0);
        assert_eq!(gate.check(A, W, t0 + secs(3)), Ok(()));
        gate.arm(A, W, secs(5), t0);
        assert_eq!(gate.check(A, W, t0 + secs(3)), Err(cooldown(2)));
    }

    #[test]
    fn check_never_writes() {
        let gate = CooldownGate::default();
        let t0 = Instant::now();
        gate.arm(A, W, secs(5), t0);
        assert_eq!(gate.len(), 1);
        assert!(gate.check(A, W, t0 + secs(1)).is_err());
        assert_eq!(gate.len(), 1);
        assert!(gate.check(A, W, t0 + secs(9)).is_ok());
        assert_eq!(gate.len(), 1);
        assert!(gate.check(Some(42), W, t0).is_ok());
        assert_eq!(gate.len(), 1);
    }

    #[test]
    fn ended_keys_are_dropped_on_arm() {
        let gate = CooldownGate::default();
        let t0 = Instant::now();
        gate.arm(A, W, secs(1), t0);
        gate.arm(Some(42), W, secs(5), t0 + secs(2));
        assert_eq!(gate.len(), 1);
    }

    #[test]
    fn a_refusal_through_the_matcher_arms_the_gate() {
        const T: u64 = 1_790_000_005;
        let call = 0x8000_0021;
        let (mut matcher, gate, t0) = (
            VerdictMatcher::default(),
            CooldownGate::default(),
            Instant::now(),
        );
        matcher.register(A, call, W, t0);
        let refused = RpcReturn {
            call_id: call,
            err_code: 5211,
            cd_end_time: T,
            error_id: 0,
            server_time: None,
        };
        let now = t0 + ms(100);
        let got = matcher
            .on_return(A, &refused, Some(T * 1000 - 5000), now)
            .unwrap();
        assert_eq!(got.cooldown, Some(secs(5)));
        gate.arm(got.client, got.channel, got.cooldown.unwrap(), now);
        assert_eq!(gate.check(A, W, t0 + secs(2)), Err(cooldown(4)));
        assert_eq!(gate.check(A, W, t0 + ms(5100)), Ok(()));

        let party = 0x8000_0022;
        matcher.register(A, party, SendChatChannel::Party, t0);
        let accepted = RpcReturn {
            call_id: party,
            err_code: 0,
            cd_end_time: 0,
            error_id: 0,
            server_time: None,
        };
        let got = matcher.on_return(A, &accepted, None, t0 + ms(50)).unwrap();
        assert_eq!(got.cooldown, None);
        assert_eq!(gate.len(), 1);
    }
}
