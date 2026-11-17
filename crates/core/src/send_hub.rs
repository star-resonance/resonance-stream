//! Where the sniffer's Returns meet the sender that waits for them: the verdict matcher, the
//! cooldown gate, the endpoint -> client route and the wait, behind `&self` methods. Pure: no
//! clock inside (callers pass `now`), no OS calls.

use crate::capture::ReturnEvent;
use crate::game_clients::Endpoint;
use crate::send_cooldown::CooldownGate;
use crate::send_verdict::{ClientKey, Resolved, Verdict, VerdictMatcher, VERDICT_WAIT};
use resonance_types::ipc::SendVerdict;
use resonance_types::SendChatChannel;
use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

/// Most endpoint routes kept.
pub const MAX_ROUTES: usize = 16;
/// Most resolved sends kept for a waiter that has not come yet.
pub const MAX_DONE: usize = 64;

type Key = (ClientKey, u32);

#[derive(Default)]
struct State {
    matcher: VerdictMatcher,
    done: HashMap<Key, Resolved>,
    order: VecDeque<Key>,
}

#[derive(Default)]
pub struct VerdictHub {
    state: Mutex<State>,
    wake: Condvar,
    pub gate: CooldownGate,
    routes: Mutex<HashMap<Endpoint, ClientKey>>,
}

impl From<Verdict> for SendVerdict {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Sent => SendVerdict::Sent,
            Verdict::TooFrequent => SendVerdict::TooFrequent,
            Verdict::Cooldown { remaining } => SendVerdict::Cooldown {
                remaining_secs: remaining.as_secs() as u32,
            },
            Verdict::NotConfirmed => SendVerdict::NotConfirmed,
            Verdict::Other(n) => SendVerdict::Refused { code: n },
        }
    }
}

/// The endpoints of `pid`'s connections in the TCP table.
pub fn endpoints_of(conns: &[(u32, Endpoint)], pid: u32) -> Vec<Endpoint> {
    conns
        .iter()
        .filter(|(p, _)| *p == pid)
        .map(|(_, e)| *e)
        .collect()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl State {
    /// Files a resolved send for its waiter and arms the gate from `now` when it carries a
    /// cooldown.
    fn settle(&mut self, gate: &CooldownGate, r: Resolved, now: Instant) {
        if let Some(c) = r.cooldown {
            gate.arm(r.client, r.channel, c, now);
        }
        let key = (r.client, r.call_id);
        if self.done.insert(key, r).is_none() {
            self.order.push_back(key);
        }
        while self.done.len() > MAX_DONE {
            match self.order.pop_front() {
                Some(old) => {
                    self.done.remove(&old);
                }
                None => break,
            }
        }
    }

    fn take(&mut self, key: Key) -> Option<Resolved> {
        let r = self.done.remove(&key)?;
        self.order.retain(|k| *k != key);
        Some(r)
    }
}

impl VerdictHub {
    /// Remembers that `endpoints` belong to the client `key` (the newest send to that client);
    /// an older route of the same key is replaced.
    pub fn learn_route(&self, key: ClientKey, endpoints: &[Endpoint]) {
        let mut routes = lock(&self.routes);
        routes.retain(|_, k| *k != key);
        for e in endpoints {
            if routes.len() >= MAX_ROUTES && !routes.contains_key(e) {
                if let Some(victim) = routes.keys().next().copied() {
                    routes.remove(&victim);
                }
            }
            routes.insert(*e, key);
        }
    }

    pub fn route(&self, e: Endpoint) -> Option<ClientKey> {
        lock(&self.routes).get(&e).copied()
    }

    /// The cooldown gate's answer for a send about to go out.
    pub fn check(
        &self,
        key: ClientKey,
        channel: SendChatChannel,
        now: Instant,
    ) -> Result<(), Verdict> {
        self.gate.check(key, channel, now)
    }

    /// A Return the sniffer framed. Ignored when no send has taught a route to its connection.
    pub fn on_return(&self, ev: &ReturnEvent) {
        let Some(key) = self.route(ev.local) else {
            log::debug!("return {:#x}.. has no route", ev.ret.call_id >> 16);
            return;
        };
        let mut st = lock(&self.state);
        let got = st
            .matcher
            .on_return(key, &ev.ret, ev.fallback_server_ms, ev.at);
        if let Some(r) = got {
            st.settle(&self.gate, r, ev.at);
            drop(st);
            self.wake.notify_all();
        }
    }

    /// Registers a send the DLL answered with `call_id`.
    pub fn register(
        &self,
        key: ClientKey,
        call_id: u32,
        channel: SendChatChannel,
        sent_at: Instant,
    ) {
        let mut st = lock(&self.state);
        let resolved = st.matcher.register(key, call_id, channel, sent_at);
        if resolved.is_empty() {
            return;
        }
        for r in resolved {
            st.settle(&self.gate, r, sent_at);
        }
        drop(st);
        self.wake.notify_all();
    }

    /// Waits until the send's Return came or [`VERDICT_WAIT`] since `sent_at` passed.
    pub fn wait(
        &self,
        key: ClientKey,
        call_id: u32,
        now: impl Fn() -> Instant,
        sent_at: Instant,
    ) -> Verdict {
        let mut st = lock(&self.state);
        loop {
            if let Some(r) = st.take((key, call_id)) {
                return r.verdict;
            }
            let at = now();
            let deadline = sent_at + VERDICT_WAIT;
            if at >= deadline {
                for r in st.matcher.poll(at) {
                    st.settle(&self.gate, r, at);
                }
                return st
                    .take((key, call_id))
                    .map_or(Verdict::NotConfirmed, |r| r.verdict);
            }
            st = self
                .wake
                .wait_timeout(st, deadline - at)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    pub fn pending_len(&self) -> usize {
        lock(&self.state).matcher.pending_len()
    }

    /// Drops the gate keys and routes of clients that are no longer running; `None` stays.
    pub fn forget_clients(&self, live: &[u32]) {
        let keep = |k: ClientKey| k.is_none_or(|p| live.contains(&p));
        self.gate.retain_clients(keep);
        lock(&self.routes).retain(|_, k| keep(*k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rpc_return::RpcReturn;
    use std::sync::Arc;
    use std::time::Duration;

    const T: u64 = 1_790_000_005;
    const W: SendChatChannel = SendChatChannel::World;
    const A: ClientKey = Some(7);
    const C: u32 = 0x8000_0011;
    const E1: Endpoint = Endpoint {
        addr: [10, 0, 0, 5],
        port: 50000,
    };
    const E2: Endpoint = Endpoint {
        addr: [10, 0, 0, 5],
        port: 50001,
    };

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }
    fn ret(call_id: u32, err: u32, cd: u64, server_time: Option<u64>) -> RpcReturn {
        RpcReturn {
            call_id,
            err_code: err,
            cd_end_time: cd,
            error_id: 0,
            server_time,
        }
    }
    fn ev(local: Endpoint, ret: RpcReturn, fallback: Option<u64>, at: Instant) -> ReturnEvent {
        ReturnEvent {
            local,
            ret,
            fallback_server_ms: fallback,
            at,
        }
    }
    fn cooldown(n: u64) -> Verdict {
        Verdict::Cooldown { remaining: secs(n) }
    }

    #[test]
    fn verdicts_map_to_the_wire_enum() {
        assert_eq!(SendVerdict::from(Verdict::Sent), SendVerdict::Sent);
        assert_eq!(
            SendVerdict::from(Verdict::TooFrequent),
            SendVerdict::TooFrequent
        );
        assert_eq!(
            SendVerdict::from(cooldown(3)),
            SendVerdict::Cooldown { remaining_secs: 3 }
        );
        assert_eq!(
            SendVerdict::from(Verdict::NotConfirmed),
            SendVerdict::NotConfirmed
        );
        assert_eq!(
            SendVerdict::from(Verdict::Other(9999)),
            SendVerdict::Refused { code: 9999 }
        );
    }

    #[test]
    fn routes_follow_the_newest_send() {
        let hub = VerdictHub::default();
        hub.learn_route(A, &[E1]);
        assert_eq!(hub.route(E1), Some(A));
        assert_eq!(hub.route(E2), None);
        hub.learn_route(A, &[E2]);
        assert_eq!(hub.route(E1), None);
        hub.learn_route(None, &[E1]);
        assert_eq!(hub.route(E1), Some(None));
        for i in 0..17u16 {
            hub.learn_route(
                Some(100 + u32::from(i)),
                &[Endpoint {
                    addr: [1, 1, 1, 1],
                    port: i,
                }],
            );
        }
        let held = (0..17u16)
            .filter(|i| {
                hub.route(Endpoint {
                    addr: [1, 1, 1, 1],
                    port: *i,
                })
                .is_some()
            })
            .count();
        assert!(held <= MAX_ROUTES);
    }

    #[test]
    fn a_return_without_a_route_changes_nothing() {
        let hub = VerdictHub::default();
        let t0 = Instant::now();
        hub.on_return(&ev(E1, ret(C, 5211, T, None), None, t0));
        hub.register(A, C, W, t0);
        assert_eq!(
            hub.wait(A, C, || t0 + VERDICT_WAIT, t0),
            Verdict::NotConfirmed
        );
        assert_eq!(hub.check(A, W, t0 + ms(1100)), Ok(()));
    }

    #[test]
    fn a_matching_return_gives_sent_at_once() {
        let hub = VerdictHub::default();
        let t0 = Instant::now();
        hub.learn_route(A, &[E1]);
        hub.register(A, C, W, t0);
        hub.on_return(&ev(E1, ret(C, 0, 0, None), None, t0 + ms(100)));
        assert_eq!(hub.wait(A, C, || t0 + ms(100), t0), Verdict::Sent);
    }

    #[test]
    fn a_return_that_came_first_is_matched_at_register() {
        let hub = VerdictHub::default();
        let t0 = Instant::now();
        hub.learn_route(A, &[E1]);
        hub.on_return(&ev(E1, ret(C, 5211, T, None), Some(T * 1000 - 3000), t0));
        hub.register(A, C, W, t0 + ms(5));
        assert_eq!(hub.wait(A, C, || t0 + ms(6), t0 + ms(5)), cooldown(3));
    }

    #[test]
    fn a_refusal_arms_the_gate_from_the_returns_arrival() {
        let hub = VerdictHub::default();
        let t0 = Instant::now();
        hub.learn_route(A, &[E1]);
        hub.register(A, C, W, t0);
        hub.on_return(&ev(
            E1,
            ret(C, 5211, T, None),
            Some(T * 1000 - 3000),
            t0 + ms(100),
        ));
        assert_eq!(hub.wait(A, C, || t0 + ms(100), t0), cooldown(3));
        assert_eq!(hub.check(A, W, t0 + secs(2)), Err(cooldown(2)));
        assert_eq!(hub.check(A, W, t0 + ms(3099)), Err(cooldown(1)));
        assert_eq!(hub.check(A, W, t0 + ms(3100)), Ok(()));

        let hub = VerdictHub::default();
        hub.learn_route(A, &[E1]);
        hub.register(A, C, W, t0);
        hub.on_return(&ev(
            E1,
            ret(C, 0, T, None),
            Some(T * 1000 - 4000),
            t0 + ms(100),
        ));
        assert_eq!(hub.wait(A, C, || t0 + ms(100), t0), Verdict::Sent);
        assert_eq!(hub.check(A, W, t0 + secs(1)), Err(cooldown(4)));

        let hub = VerdictHub::default();
        hub.learn_route(A, &[E1]);
        hub.register(A, C, SendChatChannel::Party, t0);
        hub.on_return(&ev(E1, ret(C, 0, 0, None), None, t0 + ms(100)));
        assert_eq!(hub.wait(A, C, || t0 + ms(100), t0), Verdict::Sent);
        assert_eq!(hub.gate.len(), 0);
    }

    #[test]
    fn no_return_ends_as_not_confirmed_and_frees_the_matcher() {
        let hub = VerdictHub::default();
        let t0 = Instant::now();
        hub.register(A, C, W, t0);
        assert_eq!(
            hub.wait(A, C, || t0 + VERDICT_WAIT, t0),
            Verdict::NotConfirmed
        );
        assert_eq!(hub.pending_len(), 0);

        hub.register(A, C, W, t0);
        let calls = std::cell::Cell::new(0);
        let started = Instant::now();
        let clock = || {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                t0 + ms(999)
            } else {
                t0 + ms(1000)
            }
        };
        assert_eq!(hub.wait(A, C, clock, t0), Verdict::NotConfirmed);
        assert!(started.elapsed() < ms(200));
    }

    #[test]
    fn a_waiting_thread_hears_the_return() {
        let hub = Arc::new(VerdictHub::default());
        hub.learn_route(A, &[E1]);
        let sent_at = Instant::now();
        hub.register(A, C, W, sent_at);
        let feeder = {
            let hub = Arc::clone(&hub);
            std::thread::spawn(move || {
                std::thread::sleep(ms(50));
                hub.on_return(&ev(E1, ret(C, 0, 0, None), None, Instant::now()));
            })
        };
        let started = Instant::now();
        assert_eq!(hub.wait(A, C, Instant::now, sent_at), Verdict::Sent);
        assert!(started.elapsed() < ms(500));
        feeder.join().unwrap();
    }

    #[test]
    fn a_waiting_thread_times_out_after_the_wait() {
        let hub = VerdictHub::default();
        let sent_at = Instant::now();
        hub.register(A, C, W, sent_at);
        let started = Instant::now();
        assert_eq!(hub.wait(A, C, Instant::now, sent_at), Verdict::NotConfirmed);
        assert!(started.elapsed() >= ms(900) && started.elapsed() < secs(3));
    }

    #[test]
    fn two_clients_never_cross() {
        let hub = VerdictHub::default();
        let t0 = Instant::now();
        hub.learn_route(Some(7), &[E1]);
        hub.learn_route(Some(42), &[E2]);
        hub.register(Some(7), C, W, t0);
        hub.register(Some(42), C, W, t0);
        hub.on_return(&ev(
            E2,
            ret(C, 5211, T, None),
            Some(T * 1000 - 3000),
            t0 + ms(100),
        ));
        assert_eq!(hub.wait(Some(42), C, || t0 + ms(100), t0), cooldown(3));
        assert_eq!(
            hub.wait(Some(7), C, || t0 + VERDICT_WAIT, t0),
            Verdict::NotConfirmed
        );
        assert_eq!(hub.check(Some(42), W, t0 + secs(1)), Err(cooldown(3)));
        assert_eq!(hub.check(Some(7), W, t0 + secs(1)), Ok(()));
    }

    #[test]
    fn endpoints_of_lists_one_pids_connections() {
        let e2b = Endpoint {
            addr: [10, 0, 0, 6],
            port: 1,
        };
        let table = [(7, E1), (42, E2), (7, e2b)];
        assert_eq!(endpoints_of(&table, 7), vec![E1, e2b]);
        assert!(endpoints_of(&table, 99).is_empty());
    }

    #[test]
    fn forget_clients_drops_gone_keys_and_routes() {
        let hub = VerdictHub::default();
        let t0 = Instant::now();
        hub.gate.arm(A, W, secs(5), t0);
        hub.gate.arm(Some(42), W, secs(5), t0);
        hub.gate.arm(None, W, secs(5), t0);
        hub.learn_route(A, &[E1]);
        hub.forget_clients(&[42]);
        assert_eq!(hub.check(A, W, t0 + secs(1)), Ok(()));
        assert!(hub.check(Some(42), W, t0 + secs(1)).is_err());
        assert!(hub.check(None, W, t0 + secs(1)).is_err());
        assert_eq!(hub.route(E1), None);
    }
}
