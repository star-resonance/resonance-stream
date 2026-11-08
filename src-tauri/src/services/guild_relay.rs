//! The guild relay's worker: one thread that feeds `resonance_core::guild_relay::GuildRelay` with
//! the settings, the player's guild and the captured lines, sends its requests to the broker and
//! tells the windows when its status changes (`guild-relay-state`).
//!
//! Nothing here logs a guild ID, a URL or a line's text.

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use resonance_core::capture::Origin;
use resonance_core::guild_relay::{GuildRelay, RelayStatus, Reply};
use resonance_core::protocol::own_player::OwnPlayer;
use resonance_types::{ChatMessage, GuildRelayConfig};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// How many events wait for the worker; past that, new ones are dropped.
pub const QUEUE: usize = 1024;
/// How long the worker waits for an event before it polls the relay anyway.
const TICK: Duration = Duration::from_millis(250);
/// How long one request to the broker may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// What the rest of the app hands the worker.
#[derive(Debug)]
pub enum Event {
    Config(GuildRelayConfig),
    Player(OwnPlayer),
    Line(Box<ChatMessage>, Origin),
}

/// The app's way to the worker (Tauri managed state). Sending never blocks: when the queue is
/// full, or the worker is gone, the event is dropped.
#[derive(Clone)]
pub struct RelayHandle {
    tx: Sender<Event>,
}

impl RelayHandle {
    pub fn set_config(&self, config: GuildRelayConfig) {
        self.send(Event::Config(config));
    }

    pub fn own_player(&self, player: OwnPlayer) {
        self.send(Event::Player(player));
    }

    pub fn line(&self, chat: &ChatMessage, origin: Origin) {
        self.send(Event::Line(Box::new(chat.clone()), origin));
    }

    fn send(&self, event: Event) {
        let _ = self.tx.try_send(event);
    }
}

/// Sends one request to the broker.
pub trait Transport {
    /// `POST url` with a JSON `body`.
    fn post(&self, url: &str, body: &str) -> Reply;
}

/// The real transport: reqwest's blocking client.
pub struct Http {
    client: reqwest::blocking::Client,
}

impl Http {
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("ResonanceStream/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }
}

impl Transport for Http {
    fn post(&self, url: &str, body: &str) -> Reply {
        let response = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send();
        let Ok(response) = response else {
            return Reply::Failed;
        };
        let status = response.status().as_u16();
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        match response.text() {
            Ok(body) => Reply::Http {
                status,
                retry_after_secs,
                body,
            },
            Err(_) => Reply::Failed,
        }
    }
}

/// `Retry-After` as a whole number of seconds; the HTTP-date form is not used by the broker.
pub fn parse_retry_after(value: &str) -> Option<u64> {
    value.trim().parse().ok()
}

/// The app's worker: starts with the saved settings and tells every window about each status
/// change. Without an HTTP client the handle still works, and its events go nowhere.
pub fn start(app: &AppHandle, config: GuildRelayConfig) -> RelayHandle {
    let relay = GuildRelay::new(config).with_local_http(crate::test_env::relay_local_http());
    match Http::new() {
        Ok(http) => {
            let app = app.clone();
            spawn(relay, http, move |status| {
                let _ = app.emit(resonance_types::ipc::event::GUILD_RELAY_STATE, status);
            })
        }
        Err(_) => {
            log::warn!("[GuildRelay] No HTTP client; the relay is off for this run");
            let (tx, _) = bounded(1);
            RelayHandle { tx }
        }
    }
}

/// Starts the worker thread. It ends when every `RelayHandle` is dropped.
pub fn spawn(
    relay: GuildRelay,
    transport: impl Transport + Send + 'static,
    on_status: impl FnMut(RelayStatus) + Send + 'static,
) -> RelayHandle {
    let (tx, rx) = bounded(QUEUE);
    std::thread::Builder::new()
        .name("guild-relay".into())
        .spawn(move || run(relay, &rx, &transport, on_status))
        .expect("spawn the guild relay thread");
    RelayHandle { tx }
}

/// The worker's loop: take the events, poll the relay, send what it asks for, report a status
/// change. Returns when the channel is closed.
fn run(
    mut relay: GuildRelay,
    rx: &Receiver<Event>,
    transport: &impl Transport,
    mut on_status: impl FnMut(RelayStatus),
) {
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut last = relay.status();
    on_status(last);
    // Reports the status when it changed since the last report.
    let mut report = |relay: &GuildRelay| {
        let status = relay.status();
        if status != last {
            last = status;
            on_status(status);
        }
    };
    loop {
        match rx.recv_timeout(TICK) {
            Ok(event) => {
                apply(&mut relay, event);
                for event in rx.try_iter() {
                    apply(&mut relay, event);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if let Some(request) = relay.poll(now()) {
            // Before the request, which may take up to TIMEOUT: Settings shows "Checking" meanwhile.
            report(&relay);
            let reply = transport.post(&request.url, &request.body);
            relay.on_response(now(), request.generation, reply);
        }
        report(&relay);
    }
}

fn apply(relay: &mut GuildRelay, event: Event) {
    match event {
        Event::Config(config) => relay.set_config(config),
        Event::Player(player) => relay.on_own_player(player),
        Event::Line(chat, origin) => relay.on_line(&chat, origin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use resonance_types::{Channel, GuildRelayStatePayload};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// Answers each request with the next reply in line and remembers what was sent.
    #[derive(Clone, Default)]
    struct Fake {
        sent: Arc<Mutex<Vec<(String, String)>>>,
        replies: Arc<Mutex<Vec<Reply>>>,
    }

    impl Transport for Fake {
        fn post(&self, url: &str, body: &str) -> Reply {
            self.sent.lock().push((url.to_string(), body.to_string()));
            let mut replies = self.replies.lock();
            if replies.is_empty() {
                Reply::Failed
            } else {
                replies.remove(0)
            }
        }
    }

    fn ok(body: &str) -> Reply {
        Reply::Http {
            status: 200,
            retry_after_secs: None,
            body: body.to_string(),
        }
    }

    fn guild_line(text: &str) -> ChatMessage {
        ChatMessage {
            uid: 5,
            timestamp: 1_772_343_736,
            nickname: "길드원".to_string(),
            message: text.to_string(),
            channel: Channel::Guild,
            ..ChatMessage::default()
        }
    }

    #[test]
    fn the_worker_asks_about_the_guild_then_relays_its_live_lines_and_reports_each_status() {
        let fake = Fake::default();
        fake.replies
            .lock()
            .extend([ok(r#"{"enrolled":true}"#), ok(r#"{"accepted":1}"#)]);
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let seen = statuses.clone();
        let (tx, rx) = bounded(QUEUE);
        let config = GuildRelayConfig {
            enabled: true,
            broker_url: "https://broker.test".to_string(),
        };
        let worker = {
            let fake = fake.clone();
            std::thread::spawn(move || {
                run(GuildRelay::new(config), &rx, &fake, move |s| {
                    seen.lock().push(s)
                })
            })
        };
        let handle = RelayHandle { tx };
        handle.own_player(OwnPlayer {
            uid: 5,
            guild_id: Some(77_001),
        });
        handle.line(&guild_line("안녕"), Origin::Live);
        handle.line(&guild_line("old"), Origin::History);

        let deadline = Instant::now() + Duration::from_secs(10);
        while fake.sent.lock().len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(handle);
        worker.join().unwrap();

        let sent = fake.sent.lock().clone();
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert_eq!(sent[0].0, "https://broker.test/v1/enrolled");
        assert_eq!(sent[1].0, "https://broker.test/v1/lines");
        assert!(sent[1].1.contains("안녕") && !sent[1].1.contains("old"));
        let statuses = statuses.lock().clone();
        assert_eq!(statuses.first(), Some(&RelayStatus::WaitingForGuild));
        assert!(statuses.contains(&RelayStatus::Checking), "{statuses:?}");
        assert_eq!(statuses.last(), Some(&RelayStatus::Relaying));
        assert!(
            statuses.windows(2).all(|w| w[0] != w[1]),
            "only changes: {statuses:?}"
        );
    }

    #[test]
    fn the_handle_keeps_the_last_status_and_its_seq_for_a_window_that_opens_late() {
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let seen = emitted.clone();
        let handle = spawn(
            GuildRelay::new(GuildRelayConfig {
                enabled: true,
                broker_url: "https://broker.test".to_string(),
            }),
            Fake::default(),
            move |payload| seen.lock().push(payload),
        );
        let wait_for = |status: RelayStatus| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while handle.state().status != status && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        };

        wait_for(RelayStatus::WaitingForGuild);
        let first = GuildRelayStatePayload {
            status: RelayStatus::WaitingForGuild,
            seq: 1,
        };
        assert_eq!(handle.state(), first);

        handle.set_config(GuildRelayConfig::default());
        wait_for(RelayStatus::Off);
        let second = first.next(RelayStatus::Off);
        assert_eq!(handle.state(), second);
        assert_eq!(*emitted.lock(), [first, second]);
    }

    #[test]
    fn the_worker_stops_when_the_app_drops_its_handle() {
        let (tx, rx) = bounded(QUEUE);
        let worker = std::thread::spawn(move || {
            run(
                GuildRelay::new(GuildRelayConfig::default()),
                &rx,
                &Fake::default(),
                |_| {},
            )
        });
        drop(RelayHandle { tx });
        worker.join().unwrap();
    }

    #[test]
    fn a_full_queue_drops_events_instead_of_blocking_the_capture() {
        let (tx, _rx) = bounded(1);
        let handle = RelayHandle { tx };
        handle.line(&guild_line("a"), Origin::Live);
        handle.line(&guild_line("b"), Origin::Live); // would block forever if it waited
    }

    #[test]
    fn retry_after_is_a_whole_number_of_seconds() {
        assert_eq!(parse_retry_after("7"), Some(7));
        assert_eq!(parse_retry_after(" 120 "), Some(120));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after("-1"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    /// One request read from a stand-in broker on 127.0.0.1: (request line, headers lowercased,
    /// body). Panics after 10 s without one, so a transport that never connects fails the test
    /// instead of hanging it.
    fn serve_once(listener: TcpListener, answer: &'static str) -> (String, Vec<String>, String) {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "no request reached the stand-in");
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("{e}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut headers = Vec::new();
        let mut length = 0;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            let header = header.trim_end().to_ascii_lowercase();
            if header.is_empty() {
                break;
            }
            if let Some(n) = header.strip_prefix("content-length:") {
                length = n.trim().parse().unwrap();
            }
            headers.push(header);
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let mut stream = stream;
        stream.write_all(answer.as_bytes()).unwrap();
        (
            request_line.trim_end().to_string(),
            headers,
            String::from_utf8(body).unwrap(),
        )
    }

    #[test]
    fn the_http_transport_posts_json_and_reads_the_status_retry_after_and_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/enrolled", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            serve_once(
                listener,
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
        });
        let reply = Http::new().unwrap().post(&url, r#"{"guildId":1}"#);
        let (request_line, headers, body) = server.join().expect("the stand-in got a request");

        assert_eq!(
            reply,
            Reply::Http {
                status: 429,
                retry_after_secs: Some(7),
                body: "{}".to_string()
            }
        );
        assert_eq!(request_line, "POST /v1/enrolled HTTP/1.1");
        assert!(headers.contains(&"content-type: application/json".to_string()));
        assert!(
            headers
                .iter()
                .any(|h| h.starts_with("user-agent: resonancestream/")),
            "{headers:?}"
        );
        assert_eq!(body, r#"{"guildId":1}"#);
    }

    #[test]
    fn no_answer_is_a_failed_reply() {
        // A port that was free a moment ago: nothing listens there.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let reply = Http::new()
            .unwrap()
            .post(&format!("http://127.0.0.1:{port}/v1/lines"), "{}");
        assert_eq!(reply, Reply::Failed);
    }
}
