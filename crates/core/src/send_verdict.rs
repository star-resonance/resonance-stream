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

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_790_000_005;
    const W: SendChatChannel = SendChatChannel::World;
    const C: u32 = 0x8000_0011;
    const A: ClientKey = Some(7);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }
    fn ret(call_id: u32, err_code: u32, cd_end_time: u64, server_time: Option<u64>) -> RpcReturn {
        RpcReturn {
            call_id,
            err_code,
            cd_end_time,
            error_id: 0,
            server_time,
        }
    }
    fn cooldown(n: u64) -> Verdict {
        Verdict::Cooldown { remaining: secs(n) }
    }

    // verdict and cooldown_remaining

    #[test]
    fn an_accepted_return_is_sent() {
        for fallback in [None, Some(T * 1000 - 3000)] {
            assert_eq!(verdict(&ret(C, 0, T, None), fallback), Verdict::Sent);
        }
        assert_eq!(verdict(&ret(C, 0, 0, None), None), Verdict::Sent);
    }

    #[test]
    fn a_refusal_gives_the_cooldown_rounded_up_and_never_zero() {
        let r = ret(C, 5211, T, None);
        assert_eq!(verdict(&r, Some(T * 1000 - 3000)), cooldown(3));
        assert_eq!(verdict(&r, Some(T * 1000 - 2200)), cooldown(3));
        assert_eq!(verdict(&r, Some(T * 1000 - 2000)), cooldown(2));
        assert_eq!(verdict(&r, Some(T * 1000 + 500)), cooldown(1));
    }

    #[test]
    fn a_cooldown_over_600_s_is_a_wrong_clock() {
        let r = ret(C, 5211, T, None);
        assert_eq!(verdict(&r, Some(T * 1000 - 600_000)), cooldown(600));
        assert_eq!(verdict(&r, Some(T * 1000 - 600_001)), Verdict::TooFrequent);
        assert_eq!(verdict(&r, Some(T * 1000 - 700_000)), Verdict::TooFrequent);
    }

    #[test]
    fn a_refusal_without_an_end_or_a_server_clock_is_too_frequent() {
        assert_eq!(
            verdict(&ret(C, 5211, 0, None), Some(T * 1000)),
            Verdict::TooFrequent
        );
        assert_eq!(verdict(&ret(C, 5211, T, None), None), Verdict::TooFrequent);
    }

    #[test]
    fn the_replys_own_time_wins_over_the_fallback() {
        let r = ret(C, 5211, T, Some(T - 3));
        assert_eq!(verdict(&r, None), cooldown(3));
        assert_eq!(verdict(&r, Some(T * 1000 - 100_000)), cooldown(3));
    }

    #[test]
    fn other_codes_are_kept_and_error_id_is_not_read() {
        assert_eq!(verdict(&ret(C, 9999, 0, None), None), Verdict::Other(9999));
        assert_eq!(verdict(&ret(C, 1, T, None), None), Verdict::Other(1));
        let mut r = ret(C, 0, 0, None);
        r.error_id = 7;
        assert_eq!(verdict(&r, None), Verdict::Sent);
        let v = Verdict::Sent;
        let copy = v;
        assert_eq!(v, copy);
    }

    #[test]
    fn cooldown_remaining_is_exact_and_bounded() {
        let acc = ret(C, 0, T, None);
        assert_eq!(
            cooldown_remaining(&acc, Some(T * 1000 - 4000)),
            Some(ms(4000))
        );
        assert_eq!(
            cooldown_remaining(&acc, Some(T * 1000 - 3500)),
            Some(ms(3500))
        );
        assert_eq!(
            cooldown_remaining(&ret(C, 0, 0, None), Some(T * 1000)),
            None
        );
        assert_eq!(cooldown_remaining(&acc, Some(T * 1000 + 1)), None);
        assert_eq!(cooldown_remaining(&acc, Some(T * 1000 - 600_001)), None);
        assert_eq!(
            cooldown_remaining(&acc, Some(T * 1000 - 600_000)),
            Some(secs(600))
        );
        assert_eq!(cooldown_remaining(&acc, None), None);
        let refused = ret(C, 5211, T, None);
        assert_eq!(
            cooldown_remaining(&refused, Some(T * 1000 - 4000)),
            Some(ms(4000))
        );
        let huge = ret(C, 0, u64::MAX, None);
        assert_eq!(cooldown_remaining(&huge, Some(T * 1000)), None);
        assert_eq!(
            verdict(&ret(C, 5211, u64::MAX, None), Some(T * 1000)),
            Verdict::TooFrequent
        );
    }

    // VerdictMatcher

    fn resolved(
        client: ClientKey,
        call_id: u32,
        channel: SendChatChannel,
        verdict: Verdict,
    ) -> Resolved {
        Resolved {
            client,
            call_id,
            channel,
            verdict,
            cooldown: None,
        }
    }

    #[test]
    fn a_return_resolves_its_send_once() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        assert!(m.register(A, C, W, t0).is_empty());
        let r = ret(C, 0, 0, None);
        assert_eq!(
            m.on_return(A, &r, None, t0 + ms(120)),
            Some(resolved(A, C, W, Verdict::Sent))
        );
        assert_eq!(m.pending_len(), 0);
        assert_eq!(m.on_return(A, &r, None, t0 + ms(130)), None);
    }

    #[test]
    fn a_return_carries_its_cooldown_for_the_gate() {
        let fb = Some(T * 1000 - 3000);
        let t0 = Instant::now();
        let mut m = VerdictMatcher::default();
        m.register(A, C, W, t0);
        let got = m
            .on_return(A, &ret(C, 5211, T, None), fb, t0 + ms(50))
            .unwrap();
        assert_eq!(got.verdict, cooldown(3));
        assert_eq!(got.cooldown, Some(secs(3)));
        m.register(A, C, W, t0);
        let got = m
            .on_return(A, &ret(C, 0, T, None), fb, t0 + ms(50))
            .unwrap();
        assert_eq!((got.verdict, got.cooldown), (Verdict::Sent, Some(secs(3))));
        m.register(A, C, W, t0);
        let got = m
            .on_return(A, &ret(C, 9999, 0, None), fb, t0 + ms(50))
            .unwrap();
        assert_eq!((got.verdict, got.cooldown), (Verdict::Other(9999), None));
    }

    #[test]
    fn unknown_ids_are_ignored_and_only_the_upper_half_is_kept() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        assert_eq!(
            m.on_return(A, &ret(0x8000_0099, 5211, T, None), None, t0),
            None
        );
        assert_eq!(m.early_len(), 1);
        assert_eq!(
            m.on_return(A, &ret(0x0000_0011, 5211, T, None), None, t0),
            None
        );
        assert_eq!(m.early_len(), 1);
        m.register(A, C, W, t0);
        assert_eq!(
            m.on_return(A, &ret(0x8000_0098, 0, 0, None), None, t0),
            None
        );
        assert_eq!(m.pending_len(), 1);
    }

    #[test]
    fn a_return_that_comes_first_resolves_the_send_at_once() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        assert_eq!(m.on_return(A, &ret(C, 0, 0, None), None, t0), None);
        assert_eq!(m.early_len(), 1);
        assert_eq!(
            m.register(A, C, W, t0 + ms(5)),
            vec![resolved(A, C, W, Verdict::Sent)]
        );
        assert_eq!((m.pending_len(), m.early_len()), (0, 0));
    }

    #[test]
    fn a_stale_early_return_is_not_used() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        m.on_return(A, &ret(C, 0, 0, None), None, t0);
        assert!(m.register(A, C, W, t0 + ms(1000)).is_empty());
        assert_eq!((m.pending_len(), m.early_len()), (1, 0));

        let mut m = VerdictMatcher::default();
        m.on_return(A, &ret(C, 0, 0, None), None, t0);
        assert!(m.poll(t0 + ms(1000)).is_empty());
        assert_eq!(m.early_len(), 0);
    }

    #[test]
    fn a_send_without_a_return_times_out_once() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        m.register(A, C, W, t0);
        assert!(m.poll(t0 + ms(999)).is_empty());
        assert_eq!(m.pending_len(), 1);
        assert_eq!(
            m.poll(t0 + ms(1000)),
            vec![resolved(A, C, W, Verdict::NotConfirmed)]
        );
        assert_eq!(m.pending_len(), 0);
        assert!(m.poll(t0 + ms(1001)).is_empty());
        assert_eq!(
            m.on_return(A, &ret(C, 0, 0, None), None, t0 + ms(1002)),
            None
        );
        assert_eq!(m.early_len(), 1);
    }

    #[test]
    fn a_late_return_is_not_confirmed_but_still_tells_the_gate() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        m.register(A, C, W, t0);
        let got = m.on_return(
            A,
            &ret(C, 5211, T, None),
            Some(T * 1000 - 3000),
            t0 + ms(1500),
        );
        assert_eq!(
            got,
            Some(Resolved {
                cooldown: Some(secs(3)),
                ..resolved(A, C, W, Verdict::NotConfirmed)
            })
        );
        assert_eq!(m.pending_len(), 0);
    }

    #[test]
    fn pending_is_bounded_and_the_oldest_is_evicted() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        for i in 0..64u32 {
            assert!(m
                .register(A, 0x8000_0000 + i, W, t0 + ms(i as u64))
                .is_empty());
        }
        let out = m.register(A, 0x8000_0040, W, t0 + ms(64));
        assert_eq!(
            out,
            vec![resolved(A, 0x8000_0000, W, Verdict::NotConfirmed)]
        );
        assert_eq!(m.pending_len(), 64);
        let now = t0 + ms(70);
        assert_eq!(
            m.on_return(A, &ret(0x8000_0000, 0, 0, None), None, now),
            None
        );
        assert!(m
            .on_return(A, &ret(0x8000_0040, 0, 0, None), None, now)
            .is_some());

        let mut m = VerdictMatcher::default();
        assert!(m.register(A, C, W, t0).is_empty());
        let out = m.register(A, C, W, t0 + ms(1));
        assert_eq!(out, vec![resolved(A, C, W, Verdict::NotConfirmed)]);
        assert_eq!(m.pending_len(), 1);
    }

    #[test]
    fn early_is_bounded_and_the_oldest_is_evicted() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        for i in 0..65u32 {
            m.on_return(
                A,
                &ret(0x8000_0000 + i, 0, 0, None),
                None,
                t0 + ms(i as u64),
            );
        }
        assert_eq!(m.early_len(), 64);
        assert!(m.register(A, 0x8000_0000, W, t0 + ms(70)).is_empty());
        assert_eq!(
            m.register(A, 0x8000_0040, W, t0 + ms(70)),
            vec![resolved(A, 0x8000_0040, W, Verdict::Sent)]
        );
    }

    #[test]
    fn clients_never_cross_and_poll_goes_oldest_first() {
        let mut m = VerdictMatcher::default();
        let t0 = Instant::now();
        let (a, b) = (Some(7), Some(42));
        m.register(a, C, W, t0);
        m.register(b, C, SendChatChannel::Party, t0);
        let fb = Some(T * 1000 - 3000);
        let got = m
            .on_return(b, &ret(C, 5211, T, None), fb, t0 + ms(10))
            .unwrap();
        assert_eq!(
            (got.client, got.channel, got.verdict),
            (b, SendChatChannel::Party, cooldown(3))
        );
        assert_eq!(m.pending_len(), 1);
        assert_eq!(
            m.on_return(None, &ret(C, 0, 0, None), None, t0 + ms(10)),
            None
        );
        let got = m
            .on_return(a, &ret(C, 0, 0, None), None, t0 + ms(10))
            .unwrap();
        assert_eq!((got.client, got.verdict), (a, Verdict::Sent));

        let mut m = VerdictMatcher::default();
        for i in 0..3u32 {
            m.register(A, 0x8000_0100 + i, W, t0 + ms(i as u64));
        }
        let ids: Vec<u32> = m.poll(t0 + secs(2)).iter().map(|r| r.call_id).collect();
        assert_eq!(ids, vec![0x8000_0100, 0x8000_0101, 0x8000_0102]);
    }
}
