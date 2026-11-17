//! Which game client is which character: a roster that maps each connection's own uid (read from
//! its world-state frames) to a nickname (read from chat lines), and the display names the
//! `game_clients` command answers. Pure: no clock inside (callers pass `now`), no OS calls.
//!
//! A name is built only from the nickname and [`UNKNOWN_NAME`]: a pid, an address, a port or an
//! executable name is never part of it (DS-0007).

use resonance_types::ipc::GameClient;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Shown while a client's character is not known.
pub const UNKNOWN_NAME: &str = "이름 확인 중";
/// Most uid -> nickname entries kept; the oldest go first.
pub const NAME_CAP: usize = 2048;
/// A connection not seen for this long is forgotten (the TCP table join ignores dead ones anyway).
pub const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A connection's local end (the game client's side), IPv4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Endpoint {
    pub addr: [u8; 4],
    pub port: u16,
}

/// One connection and its character's nickname, when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientFact {
    pub local: Endpoint,
    pub nickname: Option<String>,
}

struct Conn {
    own_uid: Option<u64>,
    nickname: Option<String>,
    last_seen: Instant,
}

#[derive(Default)]
pub struct Roster {
    names: HashMap<u64, String>,
    order: VecDeque<u64>,
    conns: BTreeMap<Endpoint, Conn>,
    dirty: bool,
    last: Vec<ClientFact>,
}

impl Roster {
    pub fn new() -> Self {
        Self::default()
    }

    fn conn(&mut self, local: Endpoint, now: Instant) -> &mut Conn {
        let dirty = &mut self.dirty;
        let conn = self.conns.entry(local).or_insert_with(|| {
            *dirty = true;
            Conn {
                own_uid: None,
                nickname: None,
                last_seen: now,
            }
        });
        conn.last_seen = now;
        conn
    }

    /// A packet of this connection was seen.
    pub fn see_connection(&mut self, local: Endpoint, now: Instant) {
        self.conn(local, now);
    }

    /// The connection's own player is `uid`; its nickname is whatever `names` holds for it.
    pub fn own_uid(&mut self, local: Endpoint, uid: u64, now: Instant) {
        let name = self.names.get(&uid).cloned();
        let conn = self.conn(local, now);
        if conn.own_uid != Some(uid) {
            conn.own_uid = Some(uid);
            conn.nickname = name;
            self.dirty = true;
        }
    }

    /// A chat line from `uid` named `nickname` (`uid == 0` and blank names carry nothing).
    pub fn see_line(&mut self, uid: u64, nickname: &str) {
        let nickname = nickname.trim();
        if uid == 0 || nickname.is_empty() {
            return;
        }
        if self.names.insert(uid, nickname.to_string()).is_none() {
            self.order.push_back(uid);
            if self.order.len() > NAME_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.names.remove(&old);
                }
            }
        }
        for conn in self.conns.values_mut() {
            if conn.own_uid == Some(uid) && conn.nickname.as_deref() != Some(nickname) {
                conn.nickname = Some(nickname.to_string());
                self.dirty = true;
            }
        }
    }

    /// The snapshot (sorted by endpoint) if it differs from the last one returned; connections
    /// idle for [`CLIENT_IDLE_TIMEOUT`] are dropped first.
    pub fn take_changed(&mut self, now: Instant) -> Option<Vec<ClientFact>> {
        let before = self.conns.len();
        self.conns
            .retain(|_, c| now.saturating_duration_since(c.last_seen) < CLIENT_IDLE_TIMEOUT);
        if self.conns.len() != before {
            self.dirty = true;
        }
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        let snapshot: Vec<ClientFact> = self
            .conns
            .iter()
            .map(|(local, c)| ClientFact {
                local: *local,
                nickname: c.nickname.clone(),
            })
            .collect();
        if snapshot == self.last {
            return None;
        }
        self.last = snapshot.clone();
        Some(snapshot)
    }
}

/// A TCP table row's `dwLocalAddr` (network-order bytes in a little-endian `u32`) and
/// `dwLocalPort`.
pub fn row_endpoint(raw_addr: u32, raw_port: u32) -> Endpoint {
    Endpoint {
        addr: raw_addr.to_le_bytes(),
        port: crate::ingame_send::remote_port(raw_port),
    }
}

/// `MIB_TCP_STATE_ESTAB`.
pub fn is_established(state: u32) -> bool {
    state == 5
}

/// Known nicknames (in pid order) with an ordinal where one occurs 2 or more times:
/// "Aria (1)", "Aria (2)". The match is exact and case-sensitive; a unique nickname is unchanged.
pub fn number_duplicates(names: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for n in names {
        *seen.entry(n.as_str()).or_default() += 1;
    }
    let mut next: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    names
        .iter()
        .map(|n| {
            if seen[n.as_str()] > 1 {
                let k = next.entry(n.as_str()).or_default();
                *k += 1;
                format!("{n} ({k})")
            } else {
                n.clone()
            }
        })
        .collect()
}

/// The clients `pids` with their display names: a pid's nickname is the first one a fact gives
/// for one of its `rows` (owner pid, local end); equal nicknames read "Name (1)", "Name (2)" in pid
/// order (see [`number_duplicates`]). Unknown is [`UNKNOWN_NAME`], numbered 1, 2, ...
/// in pid order only when two or more are unknown.
pub fn client_views(
    pids: &[u32],
    rows: &[(u32, Endpoint)],
    facts: &[ClientFact],
) -> Vec<GameClient> {
    let mut pids = pids.to_vec();
    pids.sort_unstable();
    pids.dedup();
    let nicks: Vec<Option<String>> = pids
        .iter()
        .map(|pid| {
            rows.iter()
                .filter(|(owner, _)| owner == pid)
                .find_map(|(_, end)| {
                    facts
                        .iter()
                        .find(|f| f.local == *end)
                        .and_then(|f| f.nickname.as_deref())
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(str::to_string)
                })
        })
        .collect();
    let unknown = nicks.iter().filter(|n| n.is_none()).count();
    let known: Vec<String> = nicks.iter().flatten().cloned().collect();
    let mut known = number_duplicates(&known).into_iter();
    let mut ordinal = 0;
    pids.iter()
        .zip(nicks)
        .map(|(&target, nick)| GameClient {
            target,
            name: match nick {
                Some(_) => known.next().unwrap_or_default(),
                None if unknown > 1 => {
                    ordinal += 1;
                    format!("{UNKNOWN_NAME} {ordinal}")
                }
                None => UNKNOWN_NAME.to_string(),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingame_send::{pick_target, send_chat_flow, SendBackend, SendError};
    use crate::send_text::SendSpacing;
    use resonance_types::ipc::GameClient;
    use resonance_types::SendChatChannel;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const A: Endpoint = Endpoint {
        addr: [10, 0, 0, 5],
        port: 51000,
    };
    const B: Endpoint = Endpoint {
        addr: [10, 0, 0, 5],
        port: 51001,
    };

    fn fact(local: Endpoint, nick: Option<&str>) -> ClientFact {
        ClientFact {
            local,
            nickname: nick.map(str::to_string),
        }
    }

    // 1
    #[test]
    fn line_then_record_names_the_connection_once() {
        let t0 = Instant::now();
        let mut roster = Roster::new();
        roster.see_line(7, "Aria");
        roster.see_connection(A, t0);
        roster.own_uid(A, 7, t0);
        assert_eq!(roster.take_changed(t0), Some(vec![fact(A, Some("Aria"))]));
        assert_eq!(roster.take_changed(t0), None);
    }

    // 2
    #[test]
    fn record_then_line_names_it_when_the_line_arrives() {
        let t0 = Instant::now();
        let mut roster = Roster::new();
        roster.own_uid(A, 7, t0);
        assert_eq!(roster.take_changed(t0), Some(vec![fact(A, None)]));
        roster.see_line(7, "Aria");
        assert_eq!(roster.take_changed(t0), Some(vec![fact(A, Some("Aria"))]));
    }

    // 3
    #[test]
    fn two_connections_keep_their_own_names_when_records_alternate() {
        let t0 = Instant::now();
        let mut roster = Roster::new();
        roster.own_uid(A, 7, t0);
        roster.own_uid(B, 9, t0);
        roster.see_line(7, "Aria");
        roster.see_line(9, "Bob");
        assert_eq!(
            roster.take_changed(t0),
            Some(vec![fact(A, Some("Aria")), fact(B, Some("Bob"))])
        );
        for local_uid in [(A, 7), (B, 9), (A, 7), (B, 9)] {
            roster.own_uid(local_uid.0, local_uid.1, t0);
        }
        assert_eq!(roster.take_changed(t0), None);
    }

    // 4
    #[test]
    fn empty_names_are_ignored_renames_apply_and_a_new_uid_resets() {
        let t0 = Instant::now();
        let mut roster = Roster::new();
        roster.own_uid(A, 7, t0);
        roster.see_line(0, "Me");
        roster.see_line(7, "  ");
        assert_eq!(roster.take_changed(t0), Some(vec![fact(A, None)]));
        roster.see_line(7, "Aria");
        roster.see_line(7, "Ari");
        assert_eq!(roster.take_changed(t0), Some(vec![fact(A, Some("Ari"))]));
        roster.own_uid(A, 8, t0);
        assert_eq!(roster.take_changed(t0), Some(vec![fact(A, None)]));
    }

    // 5
    #[test]
    fn the_roster_is_bounded_and_forgets_idle_connections() {
        let t0 = Instant::now();
        let mut roster = Roster::new();
        roster.see_line(1, "First");
        roster.own_uid(A, 1, t0);
        for uid in 2..=(NAME_CAP as u64 + 1) {
            roster.see_line(uid, "x");
        }
        // uid 1's name is gone from the map, but A keeps what it had.
        roster.own_uid(B, 1, t0);
        assert_eq!(
            roster.take_changed(t0),
            Some(vec![fact(A, Some("First")), fact(B, None)])
        );
        let later = t0 + CLIENT_IDLE_TIMEOUT + Duration::from_secs(1);
        roster.see_connection(B, later);
        assert_eq!(roster.take_changed(later), Some(vec![fact(B, None)]));
    }

    // 8
    #[test]
    fn names_follow_the_pids_rows_and_facts() {
        let named = |pid, name: &str| GameClient {
            target: pid,
            name: name.to_string(),
        };
        let rows = [(7, A)];
        let facts = [fact(A, Some("Aria"))];
        assert_eq!(client_views(&[7], &rows, &facts), [named(7, "Aria")]);
        assert_eq!(client_views(&[7], &[], &facts), [named(7, UNKNOWN_NAME)]);
        assert_eq!(client_views(&[7], &rows, &[]), [named(7, UNKNOWN_NAME)]);
        assert_eq!(
            client_views(&[7, 42], &[], &[]),
            [named(7, "이름 확인 중 1"), named(42, "이름 확인 중 2")]
        );
        let rows2 = [(7, A), (42, B)];
        assert_eq!(
            client_views(&[7, 42], &rows2, &facts),
            [named(7, "Aria"), named(42, UNKNOWN_NAME)]
        );
        assert_eq!(
            client_views(&[7], &rows, &[fact(A, Some("  "))]),
            [named(7, UNKNOWN_NAME)]
        );
        assert_eq!(
            client_views(&[7], &[(7, B), (7, A)], &facts),
            [named(7, "Aria")]
        );
        assert_eq!(
            client_views(&[7], &rows, &[fact(B, Some("Bob"))]),
            [named(7, UNKNOWN_NAME)]
        );
        assert_eq!(
            client_views(
                &[42, 7],
                &rows2,
                &[fact(A, Some("Aria")), fact(B, Some("Bob"))]
            ),
            [named(7, "Aria"), named(42, "Bob")]
        );
    }

    #[test]
    fn equal_nicknames_get_ordinals_in_order() {
        let v =
            |a: &[&str]| number_duplicates(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(v(&["Aria", "Bob", "Aria"]), ["Aria (1)", "Bob", "Aria (2)"]);
        assert_eq!(
            v(&["Aria", "Aria", "Aria"]),
            ["Aria (1)", "Aria (2)", "Aria (3)"]
        );
        assert_eq!(v(&["Aria", "aria"]), ["Aria", "aria"]);
        assert_eq!(v(&["Aria", "Aria2"]), ["Aria", "Aria2"]);
        assert!(v(&[]).is_empty());
        assert_eq!(v(&["Aria"]), ["Aria"]);
    }

    #[test]
    fn client_views_numbers_duplicate_nicknames_by_pid() {
        let named = |pid, name: &str| GameClient {
            target: pid,
            name: name.to_string(),
        };
        let rows = [(7, A), (42, B)];
        let both = [fact(A, Some("Aria")), fact(B, Some("Aria"))];
        assert_eq!(
            client_views(&[42, 7], &rows, &both),
            [named(7, "Aria (1)"), named(42, "Aria (2)")]
        );
        assert_eq!(
            client_views(
                &[7, 42],
                &rows,
                &[fact(A, Some("Aria")), fact(B, Some("Bob"))]
            ),
            [named(7, "Aria"), named(42, "Bob")]
        );
        assert_eq!(
            client_views(&[7, 42, 50], &rows, &both),
            [
                named(7, "Aria (1)"),
                named(42, "Aria (2)"),
                named(50, UNKNOWN_NAME)
            ]
        );
        assert_eq!(
            client_views(&[7, 42, 50], &rows[..1], &both[..1]),
            [
                named(7, "Aria"),
                named(42, "이름 확인 중 1"),
                named(50, "이름 확인 중 2")
            ]
        );
        assert_eq!(
            client_views(
                &[7, 42],
                &rows,
                &[fact(A, Some("  Aria ")), fact(B, Some("Aria"))]
            ),
            [named(7, "Aria (1)"), named(42, "Aria (2)")]
        );
    }

    // 9
    #[test]
    fn a_name_never_shows_a_technical_id() {
        let views = client_views(&[4242], &[(4242, A)], &[]);
        assert_eq!(views[0].name, "이름 확인 중");
        for bad in ["4242", "10.0.0.5", "51000", "StarSEA", ":"] {
            assert!(!views[0].name.contains(bad), "{bad}");
        }
    }

    // 10
    #[test]
    fn a_table_row_becomes_an_endpoint_and_only_established_counts() {
        assert_eq!(
            row_endpoint(0x0500_000A, 0x0000_58C8),
            Endpoint {
                addr: [10, 0, 0, 5],
                port: 51288
            }
        );
        assert!(is_established(5));
        for state in [1, 2, 11, 12] {
            assert!(!is_established(state), "{state}");
        }
    }

    // 11
    struct Fake(Vec<Option<u32>>);
    impl SendBackend for Fake {
        fn send(
            &mut self,
            target: Option<u32>,
            _channel: SendChatChannel,
            _text: &str,
        ) -> Result<(), SendError> {
            self.0.push(target);
            Ok(())
        }
    }

    #[test]
    fn the_target_of_a_client_reaches_send_chat() {
        let views = client_views(&[7, 42], &[], &[]);
        assert_eq!(views.len(), 2);
        let target = views[0].target;
        assert_eq!(pick_target(&[7, 42], Some(target)), Ok(7));
        let spacing = Mutex::new(SendSpacing::default());
        let mut fake = Fake(Vec::new());
        send_chat_flow(
            true,
            &spacing,
            &mut fake,
            Some(target),
            SendChatChannel::World,
            "hi",
        )
        .unwrap();
        assert_eq!(fake.0, [Some(7)]);

        // With two clients and no target, the real backend answers ChooseClient; pick_target
        // is what decides it.
        assert_eq!(pick_target(&[7, 42], None), Err(SendError::ChooseClient));
        assert_eq!(pick_target(&[7, 42], Some(9)), Err(SendError::ClientGone));
        assert_eq!(
            SendError::ChooseClient.to_string(),
            "게임이 둘 이상 연결되어 있습니다. 보낼 게임을 골라 주세요."
        );
    }
}
