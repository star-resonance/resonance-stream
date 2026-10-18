//! The test bridge (`--bridge-url mqtt://127.0.0.1:PORT`): the app publishes what
//! it does to a local MQTT broker and obeys the commands a test publishes there.
//! Topics, payloads and the command allowlist are `resonance_core::bridge`; this
//! file only moves them.
//!
//! * every backend -> UI event (`EVENT_NAMES`) is heard with `listen_any` and
//!   published as `rs/app/event/<name>`, so no emit site changed;
//! * `rs/test/command/<name>` is parsed, run, and answered on `rs/app/ack/<id>`.
//!   Commands run one at a time, in the order they came, on a task of the async
//!   runtime: one that takes long never holds up the connection (its keep-alive).
//!
//! A bridge that is slow or missing never blocks the app: publishing only queues
//! (a full queue drops the message, with a warning), and the connection thread
//! keeps retrying. Honoured only when the test flags are (`test_env::GATE_OPEN`);
//! the payloads hold chat text, which is why the address must be loopback.

use resonance_core::bridge::{self as wire, Command, EVENT_NAMES};
use rumqttc::{Client, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::ipc::{Invoke, InvokeBody};
use tauri::{AppHandle, Listener, Manager, Runtime};

/// The connection, once the bridge is on; the events and the command tap share it.
static CLIENT: OnceLock<Client> = OnceLock::new();
/// One counter for everything published, so a test sees the order of events and commands.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// Messages that can wait for the connection thread before new ones are dropped.
const QUEUE: usize = 1024;
const RETRY: Duration = Duration::from_secs(1);

/// Starts the bridge when `--bridge-url` was given; does nothing otherwise.
pub fn start(app: &AppHandle) {
    let Some((host, port)) = crate::test_env::bridge_addr() else {
        return;
    };
    // rumqttc wants the bare address of an IPv6 host.
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let mut options = MqttOptions::new("resonance-stream-app", &host, port);
    options.set_keep_alive(Duration::from_secs(5));
    // The client's default is 10 KiB, which dropped the answer to `get-chat-history` after a busy session.
    options.set_max_packet_size(wire::MAX_PACKET_BYTES, wire::MAX_PACKET_BYTES);
    options.set_last_will(LastWill::new(
        wire::STATUS_TOPIC,
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    let (client, mut connection) = Client::new(options, QUEUE);

    let _ = CLIENT.set(client.clone());
    // Queued until the connection is up. A second one with another pid is a restart (after an update).
    publish_event(
        wire::APP_STARTED_EVENT,
        serde_json::json!({
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"),
            "exe": std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        }),
    );

    // Events: heard on any target, published in the order they arrive.
    for name in EVENT_NAMES {
        app.listen_any(name, move |event| {
            let payload = serde_json::from_str(event.payload())
                .unwrap_or_else(|_| Value::String(event.payload().to_string()));
            publish_envelope(wire::event_topic(name), name, payload);
        });
    }

    // Commands, in the order they arrive: the connection thread only queues them.
    let (commands, mut queue) = tokio::sync::mpsc::unbounded_channel::<(String, Vec<u8>)>();
    let (app, runner) = (app.clone(), client.clone());
    tauri::async_runtime::spawn(async move {
        while let Some((topic, payload)) = queue.recv().await {
            handle(&app, &runner, &topic, &payload).await;
        }
    });

    std::thread::spawn(move || {
        log::info!("[Bridge] Using the broker at {host}:{port}");
        for notification in connection.iter() {
            match notification {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    log::info!("[Bridge] Connected");
                    let _ =
                        client.try_publish(wire::STATUS_TOPIC, QoS::AtLeastOnce, true, "online");
                    let _ = client.try_subscribe(wire::COMMAND_FILTER, QoS::AtLeastOnce);
                }
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    let _ = commands.send((publish.topic, publish.payload.to_vec()));
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!("[Bridge] {e}");
                    std::thread::sleep(RETRY);
                }
            }
        }
    });
}

/// Publishes an event the app raises itself (`update-state`, `app-started`), as it would a UI event.
/// Does nothing unless the bridge is on.
pub fn publish_event(name: &str, payload: Value) {
    publish_envelope(wire::event_topic(name), name, payload);
}

/// Wraps the app's command handler: every UI -> backend `invoke` is published
/// as `rs/app/command/<name>` (with its arguments) before it runs. Does nothing
/// else, and nothing at all when the bridge is off.
pub fn tap_commands<R: Runtime>(
    inner: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if CLIENT.get().is_some() {
            let name = invoke.message.command();
            if let Some(topic) = wire::command_topic(name) {
                let payload = match invoke.message.payload() {
                    InvokeBody::Json(json) => json.clone(),
                    InvokeBody::Raw(bytes) => serde_json::json!({ "raw_bytes": bytes.len() }),
                };
                publish_envelope(topic, name, payload);
            }
        }
        inner(invoke)
    }
}

/// Queues one enveloped message; a full queue drops it, with a warning.
fn publish_envelope(topic: String, name: &str, payload: Value) {
    let Some(client) = CLIENT.get() else { return };
    let text = wire::envelope(SEQ.fetch_add(1, Ordering::SeqCst), now_ms(), name, payload);
    if let Err(e) = client.try_publish(topic, QoS::AtLeastOnce, false, text.into_bytes()) {
        log::warn!("[Bridge] Dropped {name}: {e}");
    }
}

/// Runs one command and answers it.
async fn handle(app: &AppHandle, client: &Client, topic: &str, payload: &[u8]) {
    let publish = |topic: String, text: String| {
        if let Err(e) = client.try_publish(topic, QoS::AtLeastOnce, false, text.into_bytes()) {
            log::warn!("[Bridge] Dropped an answer: {e}");
        }
    };
    let request = match wire::parse_command(topic, payload) {
        Ok(request) => request,
        Err(e) => {
            log::warn!("[Bridge] Refused {topic}: {e}");
            let answer = match e.id() {
                Some(id) => (wire::ack_topic(id), wire::ack(id, Err(&e.to_string()))),
                None => (wire::ERROR_TOPIC.to_string(), wire::error_message(&e)),
            };
            return publish(answer.0, answer.1);
        }
    };
    // The bridge starts before the rest of start-up, so that it hears every event (the translator's first states,
    // the first system messages). A command must not run on a half-started app: wait for `ready`.
    let waited = Instant::now();
    while !crate::test_env::is_ready() && waited.elapsed() < Duration::from_secs(60) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    log::info!("[Bridge] Command {:?}", request.command);
    let answer = |result: Result<(), String>| {
        let text = match &result {
            Ok(()) => wire::ack(&request.id, Ok(())),
            Err(e) => wire::ack(&request.id, Err(e)),
        };
        publish(wire::ack_topic(&request.id), text);
    };
    let answer_data = |data: Value| {
        publish(
            wire::ack_topic(&request.id),
            wire::fit_ack(&request.id, wire::ack_data(&request.id, data)),
        );
    };
    let rect_json = |rect: Option<resonance_types::WindowRect>| {
        rect.and_then(|r| serde_json::to_value(r).ok())
            .unwrap_or(Value::Null)
    };
    match &request.command {
        Command::Ping => answer(Ok(())),
        Command::Snapshot => match app.get_window("main") {
            Some(window) => answer_data(rect_json(crate::window::window_rect(&window))),
            None => answer(Err("There is no main window".into())),
        },
        Command::GrowWindow {
            min_width,
            min_height,
        } => match app.get_window("main") {
            // What the settings view does: the ack's data is the rect it replaced (null: nothing changed).
            Some(window) => answer_data(rect_json(crate::window::grow_window(
                window,
                f64::from(*min_width),
                f64::from(*min_height),
            ))),
            None => answer(Err("There is no main window".into())),
        },
        Command::CloseWindow => match app.get_window("main") {
            Some(window) => {
                answer(Ok(()));
                // The X: a close request (the app's handler restores a grown window), then the app ends.
                let _ = window.close();
            }
            None => answer(Err("There is no main window".into())),
        },
        Command::ReplayChat { path } => {
            // Reports its own problems as system messages (what `--replay-chat` does).
            crate::services::sniffer::replay::start_file(app.clone(), path);
            answer(Ok(()));
        }
        Command::StartUpdate => match crate::services::downloader::announced_update(app) {
            None => answer(Err(
                "No update has been announced; check for updates first".into()
            )),
            Some(feed) => {
                // The outcome is the `update-state` events (downloading, then downloaded or error).
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::services::downloader::download_app_update(app, feed.url).await;
                });
                answer(Ok(()));
            }
        },
        Command::RestartUpdate => {
            // On success this process ends (a new one says `app-started`); only a refusal comes back.
            if let Err(e) = crate::services::downloader::restart_to_apply_update(app.clone()) {
                answer(Err(e));
            }
        }
        // The chat-row menu's and the clear button's commands, called the way the UI's invoke reaches them.
        Command::BlockUser { uid, nickname } => {
            crate::services::sniffer::block_user_command(
                *uid,
                nickname.clone(),
                app.clone(),
                app.state(),
            );
            answer(Ok(()));
        }
        Command::UnblockUser { uid } => {
            crate::services::sniffer::unblock_user_command(*uid, app.clone(), app.state());
            answer(Ok(()));
        }
        Command::GetChatHistory => {
            let history = crate::commands::get_chat_history(app.state());
            answer_data(serde_json::to_value(history).unwrap_or(Value::Null));
        }
        Command::ClearHistory => {
            crate::commands::clear_chat_history(app.state());
            answer(Ok(()));
        }
        Command::SyncDictionary => {
            // The settings view's sync: an async command that fetches, validates, saves and installs; its error is the ack's.
            let synced = crate::services::downloader::sync_dictionary(app.clone()).await;
            answer(synced.map(|_| ()));
        }
        Command::SaveLocalDictionary { content } => {
            // The dictionary editor's save: validates, writes custom_dict.json, installs from the next job on.
            answer(crate::services::downloader::save_local_dictionary(
                app.clone(),
                content.clone(),
            ));
        }
        Command::AnnotateFurigana { texts } => {
            // The UI's own command, an async fn that only computes.
            let spans = crate::commands::annotate_furigana(texts.clone()).await;
            answer_data(serde_json::to_value(spans).unwrap_or(Value::Null));
        }
        Command::GetFavorites => {
            // What `save_favorites` tells the windows (`favorites-changed`); the UI reads the same from the config it loads.
            let favorites = app.state::<crate::AppState>().config.read().favorites();
            answer_data(serde_json::to_value(favorites).unwrap_or(Value::Null));
        }
        Command::SaveFavorites { favorites } => {
            // The popup's own command: the favorites only, the config file written, `favorites-changed` emitted. It returns
            // when the file is written, so the ack means "saved".
            crate::config::save_favorites(app.clone(), app.state(), favorites.clone());
            answer(Ok(()));
        }
        Command::RestartSniffer => {
            // Runs on a thread of its own (a pause for the OS to release the socket): ack now, the test waits for `sniffer-state`.
            crate::services::sniffer::restart_sniffer_command(app.clone());
            answer(Ok(()));
        }
        Command::OpenPopup { kind } => {
            // `open_popup` is async on purpose (a window made from a synchronous command can deadlock on Windows);
            // this runs on the async runtime too. The ack comes when the window is shown.
            answer(crate::window::open_popup(app.clone(), *kind).await);
        }
        Command::HidePopup { kind } => answer(crate::window::hide_popup(app, *kind)),
        Command::PlacePopup { kind, rect } => answer(crate::window::place_popup(app, *kind, *rect)),
        Command::SnapshotPopups => answer_data(crate::window::popups_snapshot(app)),
        Command::PinMain { on } => match app.get_window("main") {
            Some(window) => {
                crate::window::set_always_on_top(window, *on);
                answer(Ok(()));
            }
            None => answer(Err("There is no main window".into())),
        },
        Command::StartTranslator => {
            crate::commands::launch_translator(app.clone(), app.state());
            answer(Ok(()));
        }
        Command::DownloadModel => {
            // The wizard's download; its end (the return value the UI shows) comes as a `download-result` event.
            let (app, id) = (app.clone(), request.id.clone());
            tauri::async_runtime::spawn(async move {
                let result = crate::services::downloader::model::download_model(app).await;
                publish_event(
                    wire::DOWNLOAD_RESULT_EVENT,
                    serde_json::json!({
                        "id": id,
                        "what": "model",
                        "ok": result.is_ok(),
                        "error": result.err(),
                    }),
                );
            });
            answer(Ok(()));
        }
        Command::Quit => {
            answer(Ok(()));
            // Let the ack leave before the process does.
            tokio::time::sleep(Duration::from_millis(200)).await;
            app.exit(0);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
