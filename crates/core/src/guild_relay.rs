//! The guild relay: which guild lines go to the guild's broker, in what batches, and what to do
//! with each answer (#367). Pure logic: no network and no clock; the app's relay service (#371)
//! sends the requests `poll` gives and feeds the answers back with `on_response`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Origin;
    use crate::protocol::own_player::OwnPlayer;
    use resonance_types::{Channel, ChatMessage, GuildRelayConfig};

    const GUILD: u64 = 77_001;
    const URL: &str = "https://relay.example";

    fn config(enabled: bool, url: &str) -> GuildRelayConfig {
        GuildRelayConfig {
            enabled,
            broker_url: url.to_string(),
        }
    }

    fn on() -> GuildRelay {
        GuildRelay::new(config(true, URL))
    }

    fn player(guild: Option<u64>) -> OwnPlayer {
        OwnPlayer {
            uid: 424_242,
            guild_id: guild,
        }
    }

    fn line(uid: u64, timestamp: u64, text: &str) -> ChatMessage {
        ChatMessage {
            uid,
            timestamp,
            nickname: "길드원".to_string(),
            message: text.to_string(),
            channel: Channel::Guild,
            ..ChatMessage::default()
        }
    }

    fn ok(body: &str) -> Reply {
        Reply::Http {
            status: 200,
            retry_after_secs: None,
            body: body.to_string(),
        }
    }

    fn status(code: u16) -> Reply {
        Reply::Http {
            status: code,
            retry_after_secs: None,
            body: String::new(),
        }
    }

    fn body(request: &RelayRequest) -> serde_json::Value {
        serde_json::from_str(&request.body).unwrap()
    }

    fn texts(request: &RelayRequest) -> Vec<String> {
        body(request)["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["text"].as_str().unwrap().to_string())
            .collect()
    }

    /// A relay that knows its guild and has been told it is enrolled, at time 0.
    fn enrolled() -> GuildRelay {
        let mut relay = on();
        relay.on_own_player(player(Some(GUILD)));
        let ask = relay.poll(0).expect("asks first");
        relay.on_response(0, ask.generation, ok(r#"{"enrolled":true}"#));
        relay
    }

    // ---- when nothing is sent ----

    #[test]
    fn the_relay_is_off_by_default_and_sends_nothing() {
        let mut relay = GuildRelay::new(GuildRelayConfig::default());
        relay.on_own_player(player(Some(GUILD)));
        relay.on_line(&line(1, 10, "hi"), Origin::Live);
        assert_eq!(relay.poll(0), None);
        assert_eq!(relay.status(), RelayStatus::Off);
    }

    #[test]
    fn an_empty_or_plain_http_broker_url_sends_nothing() {
        for url in [
            "",
            "   ",
            "http://relay.example",
            "ftp://relay.example",
            "relay.example",
        ] {
            let mut relay = GuildRelay::new(config(true, url));
            relay.on_own_player(player(Some(GUILD)));
            assert_eq!(relay.poll(0), None, "{url:?}");
            assert_eq!(relay.status(), RelayStatus::NoBroker, "{url:?}");
        }
    }

    #[test]
    fn nothing_is_sent_until_the_guild_is_known() {
        let mut relay = on();
        assert_eq!(relay.poll(0), None);
        assert_eq!(relay.status(), RelayStatus::WaitingForGuild);
        relay.on_own_player(player(None));
        assert_eq!(relay.poll(0), None);
        assert_eq!(relay.status(), RelayStatus::WaitingForGuild);
    }

    // ---- enrollment ----

    #[test]
    fn the_relay_first_asks_whether_the_guild_is_enrolled() {
        let mut relay = GuildRelay::new(config(true, "https://relay.example/"));
        relay.on_own_player(player(Some(GUILD)));
        let ask = relay.poll(0).unwrap();
        assert_eq!(ask.url, "https://relay.example/v1/enrolled");
        assert_eq!(body(&ask), serde_json::json!({ "guildId": GUILD }));
        assert_eq!(relay.status(), RelayStatus::Checking);
        assert_eq!(relay.poll(0), None, "one request at a time");
    }

    #[test]
    fn lines_seen_before_the_answer_wait_and_go_once_the_guild_is_enrolled() {
        let mut relay = on();
        relay.on_line(&line(1, 10, "before the guild"), Origin::Live);
        relay.on_own_player(player(Some(GUILD)));
        let ask = relay.poll(0).unwrap();
        relay.on_line(&line(2, 11, "while asking"), Origin::Live);
        relay.on_response(0, ask.generation, ok(r#"{"enrolled":true}"#));
        assert_eq!(relay.status(), RelayStatus::Relaying);
        let send = relay.poll(0).unwrap();
        assert_eq!(send.url, "https://relay.example/v1/lines");
        assert_eq!(body(&send)["guildId"], GUILD);
        assert_eq!(texts(&send), ["before the guild", "while asking"]);
    }

    #[test]
    fn a_guild_that_is_not_enrolled_drops_its_lines_and_is_asked_again_after_ten_minutes() {
        let mut relay = on();
        relay.on_line(&line(1, 10, "never sent"), Origin::Live);
        relay.on_own_player(player(Some(GUILD)));
        let ask = relay.poll(0).unwrap();
        relay.on_response(0, ask.generation, ok(r#"{"enrolled":false}"#));
        assert_eq!(relay.status(), RelayStatus::NotEnrolled);
        relay.on_line(&line(2, 11, "not kept"), Origin::Live);
        assert_eq!(relay.poll(NOT_ENROLLED_RETRY_MS - 1), None);
        let again = relay.poll(NOT_ENROLLED_RETRY_MS).unwrap();
        assert!(again.url.ends_with("/v1/enrolled"));
        relay.on_response(
            NOT_ENROLLED_RETRY_MS,
            again.generation,
            ok(r#"{"enrolled":true}"#),
        );
        assert_eq!(relay.poll(NOT_ENROLLED_RETRY_MS), None, "nothing was kept");
    }

    #[test]
    fn a_new_guild_drops_the_queue_and_asks_again() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "old guild"), Origin::Live);
        relay.on_own_player(player(Some(GUILD + 1)));
        let ask = relay.poll(0).unwrap();
        assert_eq!(body(&ask), serde_json::json!({ "guildId": GUILD + 1 }));
        relay.on_response(0, ask.generation, ok(r#"{"enrolled":true}"#));
        assert_eq!(relay.poll(0), None);
    }

    #[test]
    fn the_same_player_record_again_changes_nothing() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "kept"), Origin::Live);
        relay.on_own_player(player(Some(GUILD)));
        assert_eq!(texts(&relay.poll(0).unwrap()), ["kept"]);
    }

    #[test]
    fn an_answer_to_a_request_from_before_a_settings_change_is_ignored() {
        let mut relay = on();
        relay.on_own_player(player(Some(GUILD)));
        let old = relay.poll(0).unwrap();
        relay.set_config(config(true, "https://other.example"));
        relay.on_response(0, old.generation, ok(r#"{"enrolled":true}"#));
        let ask = relay.poll(0).unwrap();
        assert_eq!(ask.url, "https://other.example/v1/enrolled");
    }

    #[test]
    fn turning_the_relay_off_drops_the_queue() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "dropped"), Origin::Live);
        relay.set_config(config(false, URL));
        assert_eq!(relay.status(), RelayStatus::Off);
        relay.set_config(config(true, URL));
        let ask = relay.poll(0).unwrap();
        relay.on_response(0, ask.generation, ok(r#"{"enrolled":true}"#));
        assert_eq!(relay.poll(0), None);
    }

    // ---- which lines ----

    #[test]
    fn only_live_guild_lines_with_a_sender_and_a_send_time_are_relayed() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "history"), Origin::History);
        relay.on_line(&line(1, 11, "replay"), Origin::Replay);
        relay.on_line(
            &ChatMessage {
                channel: Channel::World,
                ..line(1, 12, "world")
            },
            Origin::Live,
        );
        relay.on_line(&line(0, 13, "no uid"), Origin::Live);
        relay.on_line(&line(1, 0, "no time"), Origin::Live);
        relay.on_line(
            &ChatMessage {
                is_blocked: true,
                ..line(1, 14, "blocked")
            },
            Origin::Live,
        );
        relay.on_line(&line(1, 15, "live"), Origin::Live);
        let send = relay.poll(0).unwrap();
        assert_eq!(texts(&send), ["blocked", "live"]);
        let first = &body(&send)["lines"][0];
        assert_eq!(first["uid"], 1);
        assert_eq!(first["nickname"], "길드원");
        assert_eq!(first["timestamp"], 14);
    }

    #[test]
    fn long_text_and_names_are_cut_by_characters() {
        let mut relay = enrolled();
        let mut chat = line(1, 10, &"가".repeat(MAX_TEXT_CHARS + 5));
        chat.nickname = "나".repeat(MAX_NICKNAME_CHARS + 5);
        relay.on_line(&chat, Origin::Live);
        let sent = &body(&relay.poll(0).unwrap())["lines"][0];
        assert_eq!(
            sent["text"].as_str().unwrap().chars().count(),
            MAX_TEXT_CHARS
        );
        assert_eq!(
            sent["nickname"].as_str().unwrap().chars().count(),
            MAX_NICKNAME_CHARS
        );
    }

    // ---- batches ----

    #[test]
    fn lines_go_in_batches_of_at_most_twenty_at_most_every_two_seconds() {
        let mut relay = enrolled();
        for i in 1..=25 {
            relay.on_line(&line(1, i, &i.to_string()), Origin::Live);
        }
        let first = relay.poll(0).unwrap();
        assert_eq!(texts(&first).len(), MAX_BATCH);
        relay.on_response(0, first.generation, ok(r#"{"accepted":20,"duplicates":0}"#));
        assert_eq!(relay.poll(BATCH_INTERVAL_MS - 1), None);
        let second = relay.poll(BATCH_INTERVAL_MS).unwrap();
        assert_eq!(texts(&second), ["21", "22", "23", "24", "25"]);
    }

    #[test]
    fn at_most_five_hundred_lines_wait_and_the_oldest_go_first() {
        let mut relay = enrolled();
        for i in 1..=(MAX_QUEUED as u64 + 3) {
            relay.on_line(&line(1, i, &i.to_string()), Origin::Live);
        }
        assert_eq!(texts(&relay.poll(0).unwrap())[0], "4");
    }

    // ---- answers ----

    #[test]
    fn a_rejected_batch_is_dropped() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "bad"), Origin::Live);
        let send = relay.poll(0).unwrap();
        relay.on_response(0, send.generation, status(400));
        assert_eq!(relay.status(), RelayStatus::Rejected);
        relay.on_line(&line(1, 11, "good"), Origin::Live);
        assert_eq!(texts(&relay.poll(BATCH_INTERVAL_MS).unwrap()), ["good"]);
    }

    #[test]
    fn a_forbidden_batch_means_the_guild_is_no_longer_enrolled() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "x"), Origin::Live);
        let send = relay.poll(0).unwrap();
        relay.on_response(0, send.generation, status(403));
        assert_eq!(relay.status(), RelayStatus::NotEnrolled);
        assert_eq!(relay.poll(NOT_ENROLLED_RETRY_MS - 1), None);
        assert!(relay
            .poll(NOT_ENROLLED_RETRY_MS)
            .unwrap()
            .url
            .ends_with("/v1/enrolled"));
    }

    #[test]
    fn a_batch_too_big_is_halved_and_a_single_line_too_big_is_dropped() {
        let mut relay = enrolled();
        for i in 1..=20 {
            relay.on_line(&line(1, i, &i.to_string()), Origin::Live);
        }
        let mut now = 0;
        let mut sizes = Vec::new();
        loop {
            let send = relay.poll(now).unwrap();
            let size = texts(&send).len();
            sizes.push(size);
            relay.on_response(now, send.generation, status(413));
            now += BATCH_INTERVAL_MS;
            if size == 1 {
                break;
            }
        }
        assert_eq!(sizes, [20, 10, 5, 2, 1]);
        assert_eq!(
            texts(&relay.poll(now).unwrap())[0],
            "2",
            "line 1 was dropped"
        );
    }

    #[test]
    fn a_full_broker_is_retried_after_the_time_it_asks_for() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "later"), Origin::Live);
        let send = relay.poll(0).unwrap();
        let busy = Reply::Http {
            status: 429,
            retry_after_secs: Some(30),
            body: String::new(),
        };
        relay.on_response(0, send.generation, busy);
        assert_eq!(relay.poll(29_999), None);
        assert_eq!(texts(&relay.poll(30_000).unwrap()), ["later"]);
    }

    #[test]
    fn server_and_network_errors_back_off_from_two_seconds_doubling_to_five_minutes() {
        let mut relay = enrolled();
        relay.on_line(&line(1, 10, "kept"), Origin::Live);
        let mut now = 0;
        let mut send = relay.poll(now).unwrap();
        let mut waits = Vec::new();
        for i in 0..10 {
            let reply = if i % 2 == 0 {
                status(503)
            } else {
                Reply::Failed
            };
            relay.on_response(now, send.generation, reply);
            assert_eq!(relay.status(), RelayStatus::Retrying);
            let mut wait = 1;
            send = loop {
                if let Some(next) = relay.poll(now + wait) {
                    break next;
                }
                wait += 1;
            };
            waits.push(wait);
            now += wait;
        }
        assert_eq!(
            waits,
            [2_000, 4_000, 8_000, 16_000, 32_000, 64_000, 128_000, 256_000, 300_000, 300_000]
        );
        assert_eq!(texts(&send), ["kept"]);
        relay.on_response(now, send.generation, ok(r#"{"accepted":1,"duplicates":0}"#));
        assert_eq!(relay.status(), RelayStatus::Relaying);
    }

    #[test]
    fn an_unreachable_broker_is_asked_again_with_the_same_backoff() {
        let mut relay = on();
        relay.on_own_player(player(Some(GUILD)));
        let ask = relay.poll(0).unwrap();
        relay.on_response(0, ask.generation, Reply::Failed);
        assert_eq!(relay.status(), RelayStatus::Retrying);
        assert_eq!(relay.poll(BACKOFF_START_MS - 1), None);
        assert!(relay
            .poll(BACKOFF_START_MS)
            .unwrap()
            .url
            .ends_with("/v1/enrolled"));
    }
}
