//! Tests first (F-MC-2): the roster and the naming of game clients. The code follows in the next commit.

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
