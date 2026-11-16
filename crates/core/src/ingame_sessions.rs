//! One in-game session per game process, kept in a map by pid.

#[cfg(test)]
mod tests {
    use super::Sessions;
    use crate::ingame_send::SendError;

    #[test]
    fn a_session_is_connected_once_per_pid() {
        let mut sessions: Sessions<&str> = Sessions::new();
        let mut runs = 0;
        for _ in 0..3 {
            let got = sessions.get_or_connect(42, || {
                runs += 1;
                Ok("a")
            });
            assert_eq!(got.map(|s| *s), Ok("a"));
        }
        assert_eq!(runs, 1);
        let got = sessions.get_or_connect(7, || {
            runs += 1;
            Ok("b")
        });
        assert_eq!(got.map(|s| *s), Ok("b"));
        assert_eq!(runs, 2);
    }

    #[test]
    fn a_failed_connect_stores_nothing_and_is_retried() {
        let mut sessions: Sessions<u8> = Sessions::new();
        let got = sessions.get_or_connect(42, || Err(SendError::NoConnect));
        assert_eq!(got.map(|s| *s), Err(SendError::NoConnect));
        assert!(sessions.is_empty());
        let got = sessions.get_or_connect(42, || Ok(1));
        assert_eq!(got.map(|s| *s), Ok(1));
    }

    #[test]
    fn retain_live_drops_the_pids_no_longer_listed() {
        let mut sessions: Sessions<u8> = Sessions::new();
        sessions.get_or_connect(42, || Ok(1)).unwrap();
        sessions.get_or_connect(7, || Ok(2)).unwrap();
        assert_eq!(sessions.retain_live(&[7]), vec![42]);
        let mut again = 0;
        sessions
            .get_or_connect(7, || {
                again += 1;
                Ok(9)
            })
            .unwrap();
        assert_eq!(again, 0);
    }

    #[test]
    fn drop_pid_forgets_one_session() {
        let mut sessions: Sessions<u8> = Sessions::new();
        sessions.get_or_connect(42, || Ok(1)).unwrap();
        sessions.drop_pid(42);
        assert!(sessions.is_empty());
    }

    #[test]
    fn take_all_returns_every_session_in_pid_order() {
        let mut sessions: Sessions<u8> = Sessions::new();
        sessions.get_or_connect(42, || Ok(1)).unwrap();
        sessions.get_or_connect(7, || Ok(2)).unwrap();
        assert_eq!(sessions.take_all(), vec![(7, 2), (42, 1)]);
        assert!(sessions.is_empty());
    }
}
