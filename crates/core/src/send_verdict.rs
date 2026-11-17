//! The server's verdict on a send: the [`Verdict`] a Return means, and a [`VerdictMatcher`] that
//! pairs each send's `call_id` with its Return within [`VERDICT_WAIT`].
//!
//! Pure: no clock inside (every call gets `now`), no wiring. The Return arrives without a pid or
//! target, so the caller gives the key a send was registered under.

use crate::protocol::rpc_return::{RpcReturn, SendOutcome};
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

/// Call ids of the DLL are in the upper half of `u32`; a Return below it is the game's own call.
const DLL_CALL_IDS: u32 = 0x8000_0000;
const MAX_COOLDOWN_MS: u64 = MAX_COOLDOWN.as_millis() as u64;

/// Milliseconds from the server's now to the Return's `cd_end_time`, 0 when it is past. `None`
/// when there is no end, the server's now is unknown (the PC's clock is never a guess) or the
/// arithmetic overflows.
fn cooldown_left_ms(ret: &RpcReturn, fallback_server_ms: Option<u64>) -> Option<u64> {
    if ret.cd_end_time == 0 {
        return None;
    }
    let now = ret
        .server_time
        .and_then(|s| s.checked_mul(1000))
        .or(fallback_server_ms)?;
    let end = ret.cd_end_time.checked_mul(1000)?;
    Some(end.saturating_sub(now))
}

/// The verdict of a Return. `fallback_server_ms` is the server's now (Unix ms) when the Return
/// carries none of its own. Reads `err_code` only (`error_id` is not looked at).
pub fn verdict(ret: &RpcReturn, fallback_server_ms: Option<u64>) -> Verdict {
    match ret.outcome() {
        SendOutcome::Accepted => Verdict::Sent,
        SendOutcome::Other(n) => Verdict::Other(n),
        SendOutcome::CooldownNotEnded => match cooldown_left_ms(ret, fallback_server_ms) {
            Some(left) if left <= MAX_COOLDOWN_MS => Verdict::Cooldown {
                remaining: Duration::from_secs(left.div_ceil(1000).max(1)),
            },
            _ => Verdict::TooFrequent,
        },
    }
}

/// The exact time until the Return's `cd_end_time`; `Some` only for a plausible future end
/// (up to [`MAX_COOLDOWN`]). Applies to an accepted reply as well as a refusal.
pub fn cooldown_remaining(ret: &RpcReturn, fallback_server_ms: Option<u64>) -> Option<Duration> {
    cooldown_left_ms(ret, fallback_server_ms)
        .filter(|left| (1..=MAX_COOLDOWN_MS).contains(left))
        .map(Duration::from_millis)
}

type Key = (ClientKey, u32);

/// Pairs sends with their Returns. No clock inside: every call gets `now`. Each map holds at
/// most [`MAX_TRACKED`] entries.
#[derive(Debug, Default)]
pub struct VerdictMatcher {
    pending: HashMap<Key, (SendChatChannel, Instant)>,
    /// Returns of the DLL's id range that came before their send was registered:
    /// the Return, the fallback it came with, its arrival.
    early: HashMap<Key, (RpcReturn, Option<u64>, Instant)>,
}

impl VerdictMatcher {
    /// Stores a send. Returns what resolved at once: the send's earlier entry under the same key
    /// or the oldest one when full (`NotConfirmed`), or the send itself when its Return came
    /// first.
    pub fn register(
        &mut self,
        client: ClientKey,
        call_id: u32,
        channel: SendChatChannel,
        sent_at: Instant,
    ) -> Vec<Resolved> {
        let key = (client, call_id);
        let mut out = Vec::new();
        if let Some((old_channel, _)) = self.pending.remove(&key) {
            out.push(not_confirmed(key, old_channel));
        }
        if let Some((ret, fallback, arrived)) = self.early.remove(&key) {
            if sent_at.saturating_duration_since(arrived) < VERDICT_WAIT {
                out.push(Resolved {
                    client,
                    call_id,
                    channel,
                    verdict: verdict(&ret, fallback),
                    cooldown: cooldown_remaining(&ret, fallback),
                });
                return out;
            }
        }
        if self.pending.len() >= MAX_TRACKED {
            let oldest = self
                .pending
                .iter()
                .min_by_key(|((_, id), (_, at))| (*at, *id))
                .map(|(k, _)| *k);
            if let Some(k) = oldest {
                if let Some((c, _)) = self.pending.remove(&k) {
                    out.push(not_confirmed(k, c));
                }
            }
        }
        self.pending.insert(key, (channel, sent_at));
        out
    }

    /// Feeds a Return seen on `client`'s stream. A pending send resolves with its verdict, or
    /// as `NotConfirmed` when it already waited [`VERDICT_WAIT`] (the cooldown is still taken
    /// from the Return). An unknown id in the DLL's range waits in `early`; any other is dropped.
    pub fn on_return(
        &mut self,
        client: ClientKey,
        ret: &RpcReturn,
        fallback_server_ms: Option<u64>,
        now: Instant,
    ) -> Option<Resolved> {
        let key = (client, ret.call_id);
        let Some((channel, sent_at)) = self.pending.remove(&key) else {
            if ret.call_id >= DLL_CALL_IDS {
                if self.early.len() >= MAX_TRACKED && !self.early.contains_key(&key) {
                    let oldest = self
                        .early
                        .iter()
                        .min_by_key(|((_, id), (_, _, at))| (*at, *id))
                        .map(|(k, _)| *k);
                    if let Some(k) = oldest {
                        self.early.remove(&k);
                    }
                }
                self.early.insert(key, (*ret, fallback_server_ms, now));
            }
            return None;
        };
        let verdict = if now.saturating_duration_since(sent_at) >= VERDICT_WAIT {
            Verdict::NotConfirmed
        } else {
            verdict(ret, fallback_server_ms)
        };
        Some(Resolved {
            client,
            call_id: ret.call_id,
            channel,
            verdict,
            cooldown: cooldown_remaining(ret, fallback_server_ms),
        })
    }

    /// Ends every send that waited [`VERDICT_WAIT`] as `NotConfirmed`, oldest first, and drops
    /// early Returns of that age.
    pub fn poll(&mut self, now: Instant) -> Vec<Resolved> {
        let mut expired: Vec<(Instant, Key)> = self
            .pending
            .iter()
            .filter(|(_, (_, at))| now.saturating_duration_since(*at) >= VERDICT_WAIT)
            .map(|(k, (_, at))| (*at, *k))
            .collect();
        expired.sort_by_key(|(at, (_, id))| (*at, *id));
        self.early
            .retain(|_, (_, _, at)| now.saturating_duration_since(*at) < VERDICT_WAIT);
        expired
            .into_iter()
            .filter_map(|(_, k)| self.pending.remove(&k).map(|(c, _)| not_confirmed(k, c)))
            .collect()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn early_len(&self) -> usize {
        self.early.len()
    }
}

fn not_confirmed((client, call_id): Key, channel: SendChatChannel) -> Resolved {
    Resolved {
        client,
        call_id,
        channel,
        verdict: Verdict::NotConfirmed,
        cooldown: None,
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
