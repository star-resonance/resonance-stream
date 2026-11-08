//! The guild relay: which guild lines go to the guild's broker, in what batches, and what to do
//! with each answer (#367). Pure logic: no network and no clock; the app's relay service (#371)
//! sends the requests `poll` gives and feeds the answers back with `on_response`.
//!
//! Nothing but the enrollment question leaves the PC until the broker has said the guild is
//! enrolled; a line seen before that waits in memory and is dropped if the answer is no.

use crate::capture::Origin;
use crate::protocol::own_player::OwnPlayer;
use crate::test_env::is_local_http_url;
pub use resonance_types::RelayStatus;
use resonance_types::{Channel, ChatMessage, GuildRelayConfig};
use std::collections::VecDeque;

/// The broker used when the setting is empty: `GUILD_RELAY_BROKER_URL` when the app is built
/// (CI passes the repository variable of that name, the Worker deployed in #369), else none.
/// Public, not a secret: the guilds a broker relays are listed in its own secret.
pub const DEFAULT_BROKER_URL: &str = match option_env!("GUILD_RELAY_BROKER_URL") {
    Some(url) => url,
    None => "",
};

/// Lines in one `/v1/lines` request.
pub const MAX_BATCH: usize = 20;
/// The shortest time between two `/v1/lines` requests, in ms.
pub const BATCH_INTERVAL_MS: u64 = 2_000;
/// Lines kept waiting at most; the oldest is dropped first.
pub const MAX_QUEUED: usize = 500;
/// Longer text is cut to this many characters.
pub const MAX_TEXT_CHARS: usize = 1_000;
/// Longer names are cut to this many characters.
pub const MAX_NICKNAME_CHARS: usize = 32;
/// After a guild is found not enrolled, the time before asking again, in ms.
pub const NOT_ENROLLED_RETRY_MS: u64 = 10 * 60 * 1_000;
/// The first wait after a server or network error, in ms; it doubles on each further error.
pub const BACKOFF_START_MS: u64 = 2_000;
/// The longest wait after errors, in ms.
pub const BACKOFF_MAX_MS: u64 = 5 * 60 * 1_000;

/// One HTTP request for the relay service to send: `POST url` with a JSON `body`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayRequest {
    pub url: String,
    pub body: String,
    /// Passed back to `on_response`, so an answer to a request made before a settings or guild
    /// change is ignored.
    pub generation: u64,
}

/// The broker's answer to a `RelayRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Http {
        status: u16,
        /// The `Retry-After` header, in seconds.
        retry_after_secs: Option<u64>,
        body: String,
    },
    /// No answer: the connection failed or timed out.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Enrollment {
    Unknown,
    Enrolled,
    NotEnrolled { ask_again_at: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InFlight {
    Enrolled,
    Lines(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelayLine {
    uid: u64,
    nickname: String,
    text: String,
    timestamp: u64,
}

pub struct GuildRelay {
    config: GuildRelayConfig,
    default_url: String,
    allow_local_http: bool,
    guild: Option<u64>,
    enrollment: Enrollment,
    queue: VecDeque<RelayLine>,
    in_flight: Option<InFlight>,
    generation: u64,
    batch_size: usize,
    /// No request before this time.
    next_at: u64,
    /// The last wait after an error; `None` after a success.
    backoff_ms: Option<u64>,
    rejected: bool,
}

impl GuildRelay {
    pub fn new(config: GuildRelayConfig) -> Self {
        Self {
            config,
            default_url: DEFAULT_BROKER_URL.to_string(),
            allow_local_http: false,
            guild: None,
            enrollment: Enrollment::Unknown,
            queue: VecDeque::new(),
            in_flight: None,
            generation: 0,
            batch_size: MAX_BATCH,
            next_at: 0,
            backoff_ms: None,
            rejected: false,
        }
    }

    /// The broker an empty setting means (tests; the app uses `DEFAULT_BROKER_URL`).
    pub fn with_default_url(mut self, url: &str) -> Self {
        self.default_url = url.to_string();
        self
    }

    /// Also accept `http://` to this machine (`127.0.0.1`, `localhost`, `[::1]`): test runs
    /// against a stand-in broker.
    pub fn with_local_http(mut self, allow: bool) -> Self {
        self.allow_local_http = allow;
        self
    }

    /// New settings. A change starts over: the guild is asked about again, and answers to
    /// requests already sent are ignored. Turning the relay off drops the waiting lines.
    pub fn set_config(&mut self, config: GuildRelayConfig) {
        if config == self.config {
            return;
        }
        self.config = config;
        self.restart();
        if !self.config.enabled {
            self.queue.clear();
        }
    }

    /// The player's record from world state. A change of guild starts over; leaving a guild (for
    /// none or another) also drops the waiting lines, which were seen in the old one. Lines
    /// seen before the first record are kept: they are the guild's the record names.
    pub fn on_own_player(&mut self, player: OwnPlayer) {
        if player.guild_id == self.guild {
            return;
        }
        if self.guild.is_some() {
            self.queue.clear();
        }
        self.guild = player.guild_id;
        self.restart();
    }

    /// A new chat line. Only live GUILD lines with a sender and a send time are kept, and only
    /// while the relay is on and the guild is not known to be refused. Block lists are ignored.
    pub fn on_line(&mut self, chat: &ChatMessage, origin: Origin) {
        if self.base_url().is_none()
            || origin != Origin::Live
            || chat.channel != Channel::Guild
            || chat.uid == 0
            || chat.timestamp == 0
            || matches!(self.enrollment, Enrollment::NotEnrolled { .. })
        {
            return;
        }
        if self.queue.len() == MAX_QUEUED {
            self.queue.pop_front();
        }
        self.queue.push_back(RelayLine {
            uid: chat.uid,
            nickname: chat.nickname.chars().take(MAX_NICKNAME_CHARS).collect(),
            text: chat.message.chars().take(MAX_TEXT_CHARS).collect(),
            timestamp: chat.timestamp,
        });
    }

    /// The next request to send at time `now` (ms), if any. One at a time: nothing more until
    /// its answer comes back through `on_response`.
    pub fn poll(&mut self, now: u64) -> Option<RelayRequest> {
        let base = self.base_url()?.to_string();
        let guild = self.guild?;
        if self.in_flight.is_some() || now < self.next_at {
            return None;
        }
        let (path, body, in_flight) = match self.enrollment {
            Enrollment::NotEnrolled { ask_again_at } if now < ask_again_at => return None,
            Enrollment::Unknown | Enrollment::NotEnrolled { .. } => {
                self.enrollment = Enrollment::Unknown;
                let body = serde_json::json!({ "guildId": guild });
                ("/v1/enrolled", body, InFlight::Enrolled)
            }
            Enrollment::Enrolled => {
                if self.queue.is_empty() {
                    return None;
                }
                let count = self.batch_size.min(self.queue.len());
                let lines: Vec<_> = self
                    .queue
                    .iter()
                    .take(count)
                    .map(|l| {
                        serde_json::json!({
                            "uid": l.uid,
                            "nickname": l.nickname,
                            "text": l.text,
                            "timestamp": l.timestamp,
                        })
                    })
                    .collect();
                let body = serde_json::json!({ "guildId": guild, "lines": lines });
                ("/v1/lines", body, InFlight::Lines(count))
            }
        };
        self.in_flight = Some(in_flight);
        Some(RelayRequest {
            url: format!("{base}{path}"),
            body: body.to_string(),
            generation: self.generation,
        })
    }

    /// The answer to the request `poll` gave with this `generation`, at time `now` (ms).
    pub fn on_response(&mut self, now: u64, generation: u64, reply: Reply) {
        if generation != self.generation {
            return;
        }
        let Some(in_flight) = self.in_flight.take() else {
            return;
        };
        let (status, retry_after_secs, body) = match reply {
            Reply::Http {
                status,
                retry_after_secs,
                body,
            } => (status, retry_after_secs, body),
            Reply::Failed => (0, None, String::new()),
        };
        match (in_flight, status) {
            (_, 403) => self.refused(now),
            (InFlight::Enrolled, 200) => {
                self.succeeded(now);
                let enrolled = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["enrolled"].as_bool());
                match enrolled {
                    Some(true) => self.enrollment = Enrollment::Enrolled,
                    Some(false) => self.refused(now),
                    None => self.failed(now, None),
                }
            }
            (InFlight::Lines(count), 200) => {
                self.queue.drain(..count.min(self.queue.len()));
                self.succeeded(now);
                self.next_at = now + BATCH_INTERVAL_MS;
            }
            (InFlight::Lines(count), 400) => {
                self.queue.drain(..count.min(self.queue.len()));
                self.backoff_ms = None;
                self.rejected = true;
                self.next_at = now + BATCH_INTERVAL_MS;
            }
            (InFlight::Lines(count), 413) => {
                if count <= 1 {
                    self.queue.pop_front();
                } else {
                    self.batch_size = count / 2;
                }
                self.next_at = now + BATCH_INTERVAL_MS;
            }
            (_, 429) => self.failed(now, retry_after_secs.map(|s| s * 1_000)),
            _ => self.failed(now, None),
        }
    }

    pub fn status(&self) -> RelayStatus {
        if !self.config.enabled {
            return RelayStatus::Off;
        }
        if self.base_url().is_none() {
            return RelayStatus::NoBroker;
        }
        if self.guild.is_none() {
            return RelayStatus::WaitingForGuild;
        }
        if self.backoff_ms.is_some() {
            return RelayStatus::Retrying;
        }
        match self.enrollment {
            Enrollment::Unknown => RelayStatus::Checking,
            Enrollment::NotEnrolled { .. } => RelayStatus::NotEnrolled,
            Enrollment::Enrolled if self.rejected => RelayStatus::Rejected,
            Enrollment::Enrolled => RelayStatus::Relaying,
        }
    }

    /// The broker URL without a trailing `/`, when the relay is on and the URL is `https://`
    /// (or local `http://` when allowed). An empty setting means the default.
    fn base_url(&self) -> Option<&str> {
        if !self.config.enabled {
            return None;
        }
        let setting = self.config.broker_url.trim();
        let url = if setting.is_empty() {
            self.default_url.trim()
        } else {
            setting
        }
        .trim_end_matches('/');
        let https = url
            .get(..8)
            .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
            && url.len() > 8;
        (https || (self.allow_local_http && is_local_http_url(url))).then_some(url)
    }

    fn restart(&mut self) {
        self.enrollment = Enrollment::Unknown;
        self.in_flight = None;
        self.generation += 1;
        self.batch_size = MAX_BATCH;
        self.next_at = 0;
        self.backoff_ms = None;
        self.rejected = false;
    }

    fn succeeded(&mut self, now: u64) {
        self.backoff_ms = None;
        self.rejected = false;
        self.next_at = now;
    }

    fn refused(&mut self, now: u64) {
        self.queue.clear();
        self.backoff_ms = None;
        self.enrollment = Enrollment::NotEnrolled {
            ask_again_at: now + NOT_ENROLLED_RETRY_MS,
        };
    }

    /// Waits `wait_ms` if the broker named a time, otherwise the next backoff step.
    fn failed(&mut self, now: u64, wait_ms: Option<u64>) {
        let backoff = self
            .backoff_ms
            .map_or(BACKOFF_START_MS, |b| (b * 2).min(BACKOFF_MAX_MS));
        self.backoff_ms = Some(backoff);
        self.next_at = now + wait_ms.unwrap_or(backoff);
    }
}

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
    fn with_no_default_an_empty_or_plain_http_broker_url_sends_nothing() {
        for url in [
            "",
            "   ",
            "http://relay.example",
            "ftp://relay.example",
            "relay.example",
        ] {
            let mut relay = GuildRelay::new(config(true, url)).with_default_url("");
            relay.on_own_player(player(Some(GUILD)));
            assert_eq!(relay.poll(0), None, "{url:?}");
            assert_eq!(relay.status(), RelayStatus::NoBroker, "{url:?}");
        }
    }

    #[test]
    fn the_built_in_default_is_the_build_time_variable_or_none() {
        let expected = option_env!("GUILD_RELAY_BROKER_URL").unwrap_or("");
        assert_eq!(DEFAULT_BROKER_URL, expected);
        let mut relay = GuildRelay::new(config(true, ""));
        relay.on_own_player(player(Some(GUILD)));
        if expected.is_empty() {
            assert_eq!(relay.poll(0), None);
            assert_eq!(relay.status(), RelayStatus::NoBroker);
        } else {
            assert_eq!(
                relay.poll(0).unwrap().url,
                format!("{expected}/v1/enrolled")
            );
        }
    }

    #[test]
    fn an_empty_broker_url_means_the_built_in_default() {
        for url in ["", "  "] {
            let mut relay =
                GuildRelay::new(config(true, url)).with_default_url("https://default.example");
            relay.on_own_player(player(Some(GUILD)));
            assert_eq!(
                relay.poll(0).unwrap().url,
                "https://default.example/v1/enrolled",
                "{url:?}"
            );
        }
        let mut own =
            GuildRelay::new(config(true, URL)).with_default_url("https://default.example");
        own.on_own_player(player(Some(GUILD)));
        assert_eq!(
            own.poll(0).unwrap().url,
            "https://relay.example/v1/enrolled"
        );
    }

    #[test]
    fn plain_http_to_this_machine_is_allowed_only_when_asked_for() {
        let local = "http://127.0.0.1:8787";
        let mut refused = GuildRelay::new(config(true, local));
        refused.on_own_player(player(Some(GUILD)));
        assert_eq!(refused.poll(0), None);
        let mut allowed = GuildRelay::new(config(true, local)).with_local_http(true);
        allowed.on_own_player(player(Some(GUILD)));
        assert_eq!(
            allowed.poll(0).unwrap().url,
            "http://127.0.0.1:8787/v1/enrolled"
        );
        let mut remote =
            GuildRelay::new(config(true, "http://relay.example")).with_local_http(true);
        remote.on_own_player(player(Some(GUILD)));
        assert_eq!(
            remote.poll(0),
            None,
            "plain http elsewhere is never allowed"
        );
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

    fn sent_nickname(nickname: &str, romaji: Option<&str>) -> String {
        let mut relay = enrolled();
        let mut chat = line(1, 10, "hi");
        chat.nickname = nickname.to_string();
        chat.nickname_romaji = romaji.map(str::to_string);
        relay.on_line(&chat, Origin::Live);
        body(&relay.poll(0).unwrap())["lines"][0]["nickname"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_name_with_a_reading_is_sent_as_the_app_shows_it() {
        assert_eq!(
            sent_nickname("アズルル", Some("Azururu")),
            "アズルル(Azururu)"
        );
    }

    #[test]
    fn a_name_whose_reading_would_not_fit_is_sent_alone() {
        let name = "ア".repeat(20);
        let reading = "A".repeat(11);
        assert_eq!(sent_nickname(&name, Some(&reading)), name);
    }

    #[test]
    fn a_name_without_a_reading_is_sent_as_it_is() {
        assert_eq!(sent_nickname("길드원", None), "길드원");
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
