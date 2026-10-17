//! The test bridge's wire rules: which MQTT topics the app uses, how an event
//! is wrapped, and which commands a test may send. Pure -- the app's client
//! (`src-tauri/src/bridge.rs`) only moves these bytes.
//!
//! One broker, one app: the app publishes what it does under `rs/app/...` and
//! obeys what a test publishes under `rs/test/command/<name>`:
//!
//! | topic | from | payload |
//! |---|---|---|
//! | `rs/app/status` (retained) | app | `online` / `offline` (last will) |
//! | `rs/app/event/<name>` | app | [`envelope`] of a backend -> UI event |
//! | `rs/app/command/<name>` | app | [`envelope`] of a UI -> backend command (its arguments) |
//! | `rs/test/command/<name>` | test | `{"id": "...", ...arguments}` ([`parse_command`]) |
//! | `rs/app/ack/<id>` | app | `{"id","ok":true}` or `{"id","ok":false,"error"}` |
//! | `rs/app/error` | app | `{"error"}` for a message that had no usable id |
//!
//! A command is on an allowlist ([`Command`]): a test can ask for what the
//! flags already do (`replay-chat`) or press a button the window has (`start-update`,
//! `restart-update`), or what a chat row's menu does (`block-user`, `unblock-user`,
//! `clear-history`), or what the setup wizard does (`download-model`, `start-translator`), or what a person does with the popup windows (`open-popup`, `hide-popup`,
//! `place-popup`, `pin-main`) -- not run arbitrary code.

use resonance_types::{FavoritesState, PopupKind, WindowRect};
use serde_json::{json, Value};
use std::fmt;
use std::path::PathBuf;

/// Every backend -> UI event name the app emits. The bridge listens to each.
pub const EVENT_NAMES: [&str; 13] = [
    "chat-message-update",
    "sniffer-state",
    "translator-state",
    "translation-event",
    "system-event",
    "packet-event",
    "favorites-changed",
    "firewall-missing",
    "download-progress",
    "popup-shown",
    "global-tab-switch",
    "tray-toggle-always-on-top",
    "tray-toggle-click-through",
];

/// Published by the app itself, not heard from the window: the update's state after every change
/// (`{"state": "none" | "available:<version>" | "downloading" | "downloaded" | "error:<reason>"}`).
pub const UPDATE_STATE_EVENT: &str = "update-state";
/// Published by the app itself when a download a test asked for ends: `{"id": <the command's id>, "what": "model",
/// "ok": bool, "error": text or null}`. (The download's own `download-progress` events are the UI's.)
pub const DOWNLOAD_RESULT_EVENT: &str = "download-result";
/// Published once when the bridge starts (`{"pid","version","exe"}`): a second one with another pid means the app restarted.
pub const APP_STARTED_EVENT: &str = "app-started";

pub const STATUS_TOPIC: &str = "rs/app/status";
pub const ERROR_TOPIC: &str = "rs/app/error";
/// The largest MQTT packet the app's client may send or accept, in bytes. The client's own default is 10 KiB, which
/// silently dropped the answer to `get-chat-history` once a session had a few hundred lines (a 108 KB ack). A whole
/// history of a busy session is a few MB at most; 16 MiB leaves room and is still a hard stop against a runaway payload.
pub const MAX_PACKET_BYTES: usize = 16 * 1024 * 1024;
/// What the app subscribes to for commands.
pub const COMMAND_FILTER: &str = "rs/test/command/+";
const COMMAND_PREFIX: &str = "rs/test/command/";

pub fn event_topic(name: &str) -> String {
    format!("rs/app/event/{name}")
}

/// Where a UI -> backend command (a Tauri `invoke`) is published; `None` when
/// the name could not be one topic level (a plugin's `plugin:x|y` is fine).
pub fn command_topic(name: &str) -> Option<String> {
    (!name.is_empty() && !name.contains(['/', '+', '#'])).then(|| format!("rs/app/command/{name}"))
}

pub fn ack_topic(id: &str) -> String {
    format!("rs/app/ack/{id}")
}

/// One event as published: a counter, the time, the event name and its payload.
pub fn envelope(seq: u64, t_ms: u64, name: &str, payload: Value) -> String {
    json!({ "seq": seq, "t_ms": t_ms, "name": name, "payload": payload }).to_string()
}

/// What a test may ask the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Only answers: is the app there and listening?
    Ping,
    /// Close the app.
    Quit,
    /// Feed the chat lines of this file in, as `--replay-chat` does.
    ReplayChat { path: PathBuf },
    /// Download the release the last update check announced (what the update dialog's button does). The
    /// outcome comes as `update-state` events.
    StartUpdate,
    /// What the settings view does when it opens: grow the main window to at least this many logical pixels
    /// (`grow_window`). The ack's `data` is the rect it replaced, or null when nothing changed.
    GrowWindow { min_width: u32, min_height: u32 },
    /// Read the main window's rect (physical pixels: `{x, y, width, height}`) into the ack's `data`.
    Snapshot,
    /// Close the main window the way its X does (the close request, then the app ends).
    CloseWindow,
    /// Install the downloaded update and restart (the dialog's 재시작). A refusal is the ack's error; a success
    /// ends this process and a new one announces itself with `app-started`.
    RestartUpdate,
    /// Put a sender on the block list, as the chat row's menu does (`block_user_command`): their rows
    /// are flagged and a `chat-message-update` says so for each. `nickname` is only the label kept in the list.
    BlockUser { uid: u64, nickname: String },
    /// Take a sender off the block list (`unblock_user_command`).
    UnblockUser { uid: u64 },
    /// The backend's chat log as the UI reads it at start-up (`get_chat_history`), in the ack's `data`.
    GetChatHistory,
    /// Empty the backend's chat and system logs (`clear_chat_history`, the clear button).
    ClearHistory,
    /// Stop the sniffer and start a fresh one, as the network troubleshooter's buttons do (`restart_sniffer_command`). The ack
    /// comes at once; the restart runs on a thread of its own and shows as `sniffer-state` events.
    RestartSniffer,
    /// Replace the favorites as the favorites popup does (`save_favorites`): the favorites only, every other setting stays; the
    /// config file is written and `favorites-changed` tells every window.
    SaveFavorites { favorites: FavoritesState },
    /// Fetch the custom dictionary and install it, as the settings view's sync does (`sync_dictionary`; the version kept
    /// as the current one is the signed metadata's). With `--dictionary-url` it reads that instead of the public gist.
    /// A failed fetch or a dictionary that does not parse is the ack's error. A `version` field, which older callers
    /// send, is ignored.
    SyncDictionary,
    /// Save the dictionary editor's text and install it from the translator's next job on (`save_local_dictionary`).
    /// A text that does not parse is the ack's error and nothing is saved.
    SaveLocalDictionary { content: String },
    /// The furigana the app gives for these lines (`annotate_furigana`, what the chat row's Study view asks): one list of
    /// `{text, reading?}` spans per line, in order, in the ack's `data`.
    AnnotateFurigana { texts: Vec<String> },
    /// The favorites the app holds (messages and tabs, as `favorites-changed` carries them), in the ack's `data`.
    GetFavorites,
    /// Start the translator as the UI does once the model and server are in place (`launch_translator`; idempotent). With
    /// `--llama-url` it uses the stand-in server and needs neither.
    StartTranslator,
    /// Open a popup window as the title bar's buttons do (`open_popup`): shown at its saved place, in front; one that is
    /// already open is only brought forward. The ack comes once the window is shown.
    OpenPopup { kind: PopupKind },
    /// Close a popup the way its X does (the close request: it hides, it is not destroyed).
    HidePopup { kind: PopupKind },
    /// Put a popup at this outer rect (physical pixels), as a person dragging and resizing it would.
    PlacePopup { kind: PopupKind, rect: WindowRect },
    /// Each popup's window as it is now, in the ack's `data`: `{"popup-cheatsheet": {exists, visible, rect, always_on_top}, ...}`,
    /// and `main` with the same, plus `labels`: every window the app has, so "never a second window" can be seen.
    SnapshotPopups,
    /// Pin the overlay over the game or let it go (`set_always_on_top`); the popups follow it.
    PinMain { on: bool },
    /// Download the translation model as the setup wizard does (`download_model`): the url, version and SHA-256 come
    /// from the verified metadata, never from the caller (fields an older caller still sends are ignored).
    /// The ack only says it started; the end is a [`DOWNLOAD_RESULT_EVENT`] carrying this command's id.
    DownloadModel,
}

/// The word a test writes for a popup (`PopupKind`'s wire name).
fn popup_name(kind: PopupKind) -> &'static str {
    match kind {
        PopupKind::CheatSheet => "cheatsheet",
        PopupKind::Favorites => "favorites",
    }
}

/// A command and the id its ack carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// The topic is not `rs/test/command/<name>`.
    NotACommandTopic(String),
    /// The payload is not a JSON object.
    BadPayload,
    /// No usable `id` (a non-empty string without `/`, `+`, `#`).
    BadId,
    /// `name` is not on the allowlist. Carries the id, to ack with.
    Unknown { id: String, name: String },
    /// An argument is missing or has the wrong type. Carries the id.
    BadArgument { id: String, reason: String },
}

impl CommandError {
    /// The id the ack goes to, when the request got far enough to have one.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Unknown { id, .. } | Self::BadArgument { id, .. } => Some(id),
            _ => None,
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotACommandTopic(t) => write!(f, "{t:?} is not a command topic"),
            Self::BadPayload => write!(f, "the payload is not a JSON object"),
            Self::BadId => write!(f, "the payload needs a string \"id\" (no / + #)"),
            Self::Unknown { name, .. } => write!(f, "unknown command {name:?}"),
            Self::BadArgument { reason, .. } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Reads a command published on `topic` with `payload`.
pub fn parse_command(topic: &str, payload: &[u8]) -> Result<Request, CommandError> {
    let name = topic
        .strip_prefix(COMMAND_PREFIX)
        .filter(|name| !name.is_empty() && !name.contains('/'))
        .ok_or_else(|| CommandError::NotACommandTopic(topic.to_string()))?;
    let Ok(Value::Object(args)) = serde_json::from_slice::<Value>(payload) else {
        return Err(CommandError::BadPayload);
    };
    let id = match args.get("id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() && !id.contains(['/', '+', '#']) => id.to_string(),
        _ => return Err(CommandError::BadId),
    };
    let command = match name {
        "ping" => Command::Ping,
        "quit" => Command::Quit,
        "snapshot" => Command::Snapshot,
        "close-window" => Command::CloseWindow,
        "grow-window" => {
            // Whole logical pixels, at least 1: a size that is missing, negative, fractional or absurd is the test's mistake.
            let size = |key: &str| {
                args.get(key)
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|&n| n > 0 && n <= 100_000)
            };
            match (size("min_width"), size("min_height")) {
                (Some(min_width), Some(min_height)) => Command::GrowWindow {
                    min_width,
                    min_height,
                },
                _ => {
                    return Err(CommandError::BadArgument {
                        id,
                        reason:
                            "grow-window needs whole \"min_width\" and \"min_height\" (1-100000)"
                                .into(),
                    })
                }
            }
        }
        "get-chat-history" => Command::GetChatHistory,
        "clear-history" => Command::ClearHistory,
        "restart-sniffer" => Command::RestartSniffer,
        "get-favorites" => Command::GetFavorites,
        "sync-dictionary" => Command::SyncDictionary,
        "save-local-dictionary" => match args.get("content").and_then(Value::as_str) {
            Some(text) => Command::SaveLocalDictionary {
                content: text.to_string(),
            },
            None => return Err(CommandError::BadArgument {
                id,
                reason:
                    "save-local-dictionary needs \"content\": the dictionary's JSON text (a string)"
                        .into(),
            }),
        },
        "annotate-furigana" => {
            let texts = args
                .get("texts")
                .and_then(Value::as_array)
                .and_then(|list| {
                    list.iter()
                        .map(|text| text.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                });
            match texts {
                Some(texts) => Command::AnnotateFurigana { texts },
                None => {
                    return Err(CommandError::BadArgument {
                        id,
                        reason: "annotate-furigana needs \"texts\": a list of strings".into(),
                    })
                }
            }
        }
        "save-favorites" => {
            // The whole state or nothing: a missing or malformed one would clear the user's favorites.
            match args
                .get("favorites")
                .cloned()
                .map(serde_json::from_value::<FavoritesState>)
            {
                Some(Ok(favorites)) => Command::SaveFavorites { favorites },
                _ => {
                    return Err(CommandError::BadArgument {
                        id,
                        reason: "save-favorites needs \"favorites\": {\"messages\": [{\"text\": ...}], \"tabs\": [{\"id\": ..., \"name\": ...}]}".into(),
                    })
                }
            }
        }
        "start-translator" => Command::StartTranslator,
        "block-user" | "unblock-user" => {
            // The game's sender ids are whole numbers, and 0 means "no sender".
            let Some(uid) = args
                .get("uid")
                .and_then(Value::as_u64)
                .filter(|&uid| uid > 0)
            else {
                return Err(CommandError::BadArgument {
                    id,
                    reason: format!("{name} needs a whole \"uid\" above 0"),
                });
            };
            if name == "unblock-user" {
                Command::UnblockUser { uid }
            } else {
                let nickname = match args.get("nickname") {
                    None => String::new(),
                    Some(Value::String(nickname)) => nickname.clone(),
                    Some(_) => {
                        return Err(CommandError::BadArgument {
                            id,
                            reason: "block-user's \"nickname\" must be a string".into(),
                        })
                    }
                };
                Command::BlockUser { uid, nickname }
            }
        }
        "download-model" => Command::DownloadModel,
        "snapshot-popups" => Command::SnapshotPopups,
        "pin-main" => match args.get("on").and_then(Value::as_bool) {
            Some(on) => Command::PinMain { on },
            None => {
                return Err(CommandError::BadArgument {
                    id,
                    reason: "pin-main needs \"on\": true or false".into(),
                })
            }
        },
        "open-popup" | "hide-popup" | "place-popup" => {
            let kind = args
                .get("kind")
                .and_then(Value::as_str)
                .and_then(|name| PopupKind::ALL.into_iter().find(|k| name == popup_name(*k)));
            let Some(kind) = kind else {
                return Err(CommandError::BadArgument {
                    id,
                    reason: format!("{name} needs a \"kind\": \"cheatsheet\" or \"favorites\""),
                });
            };
            match name {
                "open-popup" => Command::OpenPopup { kind },
                "hide-popup" => Command::HidePopup { kind },
                _ => {
                    let whole = |key: &str| {
                        args.get(key)
                            .and_then(Value::as_i64)
                            .and_then(|n| i32::try_from(n).ok())
                    };
                    let size = |key: &str| {
                        args.get(key)
                            .and_then(Value::as_u64)
                            .and_then(|n| u32::try_from(n).ok())
                            .filter(|&n| n > 0 && n <= 100_000)
                    };
                    match (whole("x"), whole("y"), size("width"), size("height")) {
                            (Some(x), Some(y), Some(width), Some(height)) => Command::PlacePopup {
                                kind,
                                rect: WindowRect { x, y, width, height },
                            },
                            _ => {
                                return Err(CommandError::BadArgument {
                                    id,
                                    reason: "place-popup needs whole \"x\" \"y\" and a \"width\" \"height\" of 1-100000".into(),
                                })
                            }
                        }
                }
            }
        }
        "start-update" => Command::StartUpdate,
        "restart-update" => Command::RestartUpdate,
        "replay-chat" => match args.get("path").and_then(Value::as_str) {
            Some(path) if !path.is_empty() => Command::ReplayChat { path: path.into() },
            _ => {
                return Err(CommandError::BadArgument {
                    id,
                    reason: "replay-chat needs a string \"path\"".into(),
                })
            }
        },
        _ => {
            return Err(CommandError::Unknown {
                id,
                name: name.to_string(),
            })
        }
    };
    Ok(Request { id, command })
}

/// The ack of a request: ok, or why not.
pub fn ack(id: &str, result: Result<(), &str>) -> String {
    match result {
        Ok(()) => json!({ "id": id, "ok": true }),
        Err(error) => json!({ "id": id, "ok": false, "error": error }),
    }
    .to_string()
}

/// An ok ack that carries an answer (`snapshot`, `grow-window`).
pub fn ack_data(id: &str, data: Value) -> String {
    json!({ "id": id, "ok": true, "data": data }).to_string()
}

/// An ack that fits one MQTT packet: `text` itself, or an error ack for `id` when it is over [`MAX_PACKET_BYTES`].
/// The client cannot send such a packet; it errors and reconnects, and the test run would only see a timeout.
pub fn fit_ack(id: &str, text: String) -> String {
    if text.len() <= MAX_PACKET_BYTES {
        return text;
    }
    let error = format!(
        "the answer is {} bytes, too large for one packet (limit {MAX_PACKET_BYTES})",
        text.len()
    );
    ack(id, Err(&error))
}

/// The payload for [`ERROR_TOPIC`].
pub fn error_message(error: &CommandError) -> String {
    match error.id() {
        Some(id) => json!({ "id": id, "error": error.to_string() }),
        None => json!({ "error": error.to_string() }),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(name: &str, payload: &str) -> Result<Request, CommandError> {
        parse_command(&format!("rs/test/command/{name}"), payload.as_bytes())
    }

    /// One chat line as `get-chat-history` carries it (about the size of a real `ChatMessage`).
    fn history_line(n: usize) -> Value {
        json!({
            "pid": n, "channel": "GUILD", "uid": 1001, "nickname": "Alice", "level": 60,
            "message": format!("チャットの一行、番号 {n}、ここに少し長めの文章が入ります"),
            "timestamp": 1_790_000_000 + n, "sequenceId": n, "isBlocked": false,
            "translated": "", "romaji": "Alice", "unknownFields": {},
        })
    }

    #[test]
    fn an_ack_of_a_whole_chat_history_fits_one_mqtt_packet() {
        // rumqttc refuses to publish more than 10 KiB unless told otherwise, so the answer to
        // `get-chat-history` after a busy session never arrived (smoke run of #200,
        // CS-restart-nodup: 108005 bytes). The limit the bridge asks for must hold a history.
        const RUMQTTC_DEFAULT: usize = 10 * 1024;
        let lines = |n: usize| ack_data("id", Value::Array((0..n).map(history_line).collect()));

        assert!(
            lines(500).len() > RUMQTTC_DEFAULT,
            "a history of 500 lines is what the default limit cannot carry"
        );
        // Four channels, each at a generous limit of 5000 lines.
        let big = lines(20_000);
        assert!(
            big.len() < MAX_PACKET_BYTES,
            "a history of 20000 lines is {} bytes, the limit is {MAX_PACKET_BYTES}",
            big.len()
        );
    }

    #[test]
    fn an_answer_over_the_packet_limit_becomes_an_error_ack() {
        // Publishing it would make the client error and reconnect, and the ack would be lost.
        let small = ack_data("a1", json!([1, 2, 3]));
        assert_eq!(
            fit_ack("a1", small.clone()),
            small,
            "a small answer is left alone"
        );

        let huge = ack_data("a2", Value::String("x".repeat(MAX_PACKET_BYTES)));
        let fitted: Value = serde_json::from_str(&fit_ack("a2", huge)).expect("json");
        assert_eq!(fitted["id"], "a2");
        assert_eq!(fitted["ok"], false);
        let error = fitted["error"].as_str().expect("an error text");
        assert!(error.contains("too large"), "{error}");
        assert!(error.contains(&MAX_PACKET_BYTES.to_string()), "{error}");
    }

    #[test]
    fn topics_are_under_rs_app() {
        assert_eq!(
            event_topic("chat-message-update"),
            "rs/app/event/chat-message-update"
        );
        assert_eq!(ack_topic("a1"), "rs/app/ack/a1");
    }

    #[test]
    fn a_ui_command_is_published_under_rs_app_command() {
        assert_eq!(
            command_topic("grow_window").as_deref(),
            Some("rs/app/command/grow_window")
        );
        assert_eq!(
            command_topic("plugin:window|set_size").as_deref(),
            Some("rs/app/command/plugin:window|set_size")
        );
        for bad in ["", "a/b", "a+", "#", "a#b"] {
            assert_eq!(command_topic(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn every_event_name_is_one_topic_level() {
        for name in EVENT_NAMES {
            assert!(
                !name.is_empty() && !name.contains(['/', '+', '#']),
                "{name}"
            );
        }
        let mut sorted = EVENT_NAMES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), EVENT_NAMES.len(), "a name is listed twice");
    }

    #[test]
    fn an_envelope_carries_seq_time_name_and_payload() {
        let text = envelope(7, 1234, "sniffer-state", json!({"running": true}));
        let v: Value = serde_json::from_str(&text).expect("json");
        assert_eq!(
            v,
            json!({"seq": 7, "t_ms": 1234, "name": "sniffer-state", "payload": {"running": true}})
        );
    }

    #[test]
    fn the_allowed_commands_parse() {
        assert_eq!(
            parse("ping", r#"{"id":"a1"}"#),
            Ok(Request {
                id: "a1".into(),
                command: Command::Ping
            })
        );
        assert_eq!(
            parse("quit", r#"{"id":"a2"}"#).map(|r| r.command),
            Ok(Command::Quit)
        );
        assert_eq!(
            parse("replay-chat", r#"{"id":"a3","path":"C:\\w\\chat.jsonl"}"#),
            Ok(Request {
                id: "a3".into(),
                command: Command::ReplayChat {
                    path: "C:\\w\\chat.jsonl".into()
                }
            })
        );
    }

    #[test]
    fn the_chat_commands_parse() {
        assert_eq!(
            parse("block-user", r#"{"id":"b","uid":1001,"nickname":"Alice"}"#).map(|r| r.command),
            Ok(Command::BlockUser {
                uid: 1001,
                nickname: "Alice".into()
            })
        );
        // The nickname is only a label in the block list: optional.
        assert_eq!(
            parse("block-user", r#"{"id":"b","uid":7}"#).map(|r| r.command),
            Ok(Command::BlockUser {
                uid: 7,
                nickname: String::new()
            })
        );
        assert_eq!(
            parse("unblock-user", r#"{"id":"u","uid":1001}"#).map(|r| r.command),
            Ok(Command::UnblockUser { uid: 1001 })
        );
        assert_eq!(
            parse("get-chat-history", r#"{"id":"h"}"#).map(|r| r.command),
            Ok(Command::GetChatHistory)
        );
        assert_eq!(
            parse("clear-history", r#"{"id":"c"}"#).map(|r| r.command),
            Ok(Command::ClearHistory)
        );
    }

    #[test]
    fn restart_sniffer_is_a_plain_command() {
        // What the network troubleshooter's buttons invoke (`restart_sniffer_command`); it takes no argument.
        assert_eq!(
            parse("restart-sniffer", r#"{"id":"r"}"#).map(|r| r.command),
            Ok(Command::RestartSniffer)
        );
        // An id is still required, as for every command.
        assert_eq!(
            parse("restart-sniffer", "{}").unwrap_err(),
            CommandError::BadId
        );
    }

    #[test]
    fn the_favorites_commands_parse() {
        use resonance_types::{FavoriteMessage, FavoriteTab, FavoritesState};
        // What the favorites popup sends (`save_favorites`): messages with the tab they are filed under, and the tabs with their ids.
        let payload = r#"{"id":"f","favorites":{"messages":[{"text":"こんにちは","note":"hello","shortcut":"Ctrl+1","tab":7},{"text":"はい"}],"tabs":[{"id":7,"name":"Greetings"}]}}"#;
        assert_eq!(
            parse("save-favorites", payload).map(|r| r.command),
            Ok(Command::SaveFavorites {
                favorites: FavoritesState {
                    messages: vec![
                        FavoriteMessage {
                            text: "こんにちは".into(),
                            note: "hello".into(),
                            shortcut: "Ctrl+1".into(),
                            tab: 7
                        },
                        // note, shortcut and tab are optional, as in the config file
                        FavoriteMessage {
                            text: "はい".into(),
                            ..Default::default()
                        }
                    ],
                    tabs: vec![FavoriteTab {
                        id: 7,
                        name: "Greetings".into()
                    }],
                }
            })
        );
        // No tabs is the default tab only.
        assert_eq!(
            parse(
                "save-favorites",
                r#"{"id":"f","favorites":{"messages":[]}}"#
            )
            .map(|r| r.command),
            Ok(Command::SaveFavorites {
                favorites: FavoritesState::default()
            })
        );
        assert_eq!(
            parse("get-favorites", r#"{"id":"g"}"#).map(|r| r.command),
            Ok(Command::GetFavorites)
        );
    }

    #[test]
    fn saving_favorites_needs_the_favorites() {
        // A missing or malformed state would otherwise clear the user's favorites: refused, with the id so the ack says why.
        for payload in [
            r#"{"id":"f"}"#,
            r#"{"id":"f","favorites":"x"}"#,
            r#"{"id":"f","favorites":{}}"#,
            r#"{"id":"f","favorites":{"messages":[{"note":"no text"}]}}"#,
            r#"{"id":"f","favorites":{"messages":[],"tabs":[{"name":"no id"}]}}"#,
        ] {
            let err = parse("save-favorites", payload).unwrap_err();
            assert_eq!(err.id(), Some("f"), "{payload}");
            assert!(matches!(err, CommandError::BadArgument { .. }), "{payload}");
        }
    }

    #[test]
    fn annotate_furigana_takes_a_list_of_lines() {
        // What the chat row's Study view asks (`annotate_furigana`): one string per line, in order.
        assert_eq!(
            parse(
                "annotate-furigana",
                r#"{"id":"a","texts":["日韓辞書","ありがとう!",""]}"#
            )
            .map(|r| r.command),
            Ok(Command::AnnotateFurigana {
                texts: vec!["日韓辞書".into(), "ありがとう!".into(), String::new()]
            })
        );
        // No lines is a valid, empty question.
        assert_eq!(
            parse("annotate-furigana", r#"{"id":"a","texts":[]}"#).map(|r| r.command),
            Ok(Command::AnnotateFurigana { texts: vec![] })
        );
    }

    #[test]
    fn annotate_furigana_refuses_anything_but_a_list_of_strings() {
        for payload in [
            r#"{"id":"a"}"#,
            r#"{"id":"a","texts":"日韓辞書"}"#,
            r#"{"id":"a","texts":[1,2]}"#,
            r#"{"id":"a","texts":["ok",null]}"#,
        ] {
            let err = parse("annotate-furigana", payload).unwrap_err();
            assert_eq!(err.id(), Some("a"), "{payload}");
            assert!(matches!(err, CommandError::BadArgument { .. }), "{payload}");
        }
    }

    #[test]
    fn the_dictionary_commands_parse() {
        // What the settings view (`sync_dictionary`, no arguments) and the dictionary editor
        // (`save_local_dictionary`, the whole JSON text) invoke.
        assert_eq!(
            parse("sync-dictionary", r#"{"id":"s"}"#).map(|r| r.command),
            Ok(Command::SyncDictionary)
        );
        // An older caller that still names a version is not refused: the app syncs what the signed metadata says.
        for payload in [
            r#"{"id":"s","version":"runbook-mock"}"#,
            r#"{"id":"s","version":7}"#,
        ] {
            assert_eq!(
                parse("sync-dictionary", payload).map(|r| r.command),
                Ok(Command::SyncDictionary),
                "{payload}"
            );
        }
        assert_eq!(
            parse(
                "save-local-dictionary",
                r#"{"id":"d","content":"{\"term\":{\"ボス\":\"보스\"}}"}"#
            )
            .map(|r| r.command),
            Ok(Command::SaveLocalDictionary {
                content: r#"{"term":{"ボス":"보스"}}"#.into()
            })
        );
        // An empty text is an empty dictionary: valid, the app decides.
        assert_eq!(
            parse("save-local-dictionary", r#"{"id":"d","content":""}"#).map(|r| r.command),
            Ok(Command::SaveLocalDictionary {
                content: String::new()
            })
        );
    }

    #[test]
    fn the_dictionary_editor_save_needs_its_text() {
        for (name, payload) in [
            ("save-local-dictionary", r#"{"id":"d"}"#),
            ("save-local-dictionary", r#"{"id":"d","content":{"a":1}}"#),
        ] {
            let err = parse(name, payload).unwrap_err();
            assert_eq!(err.id(), Some("d"));
            assert!(
                matches!(err, CommandError::BadArgument { .. }),
                "{name} {payload}"
            );
        }
    }

    #[test]
    fn blocking_needs_a_whole_uid() {
        // A uid is what the game sends: a whole number, and never 0 (0 is "no sender").
        for payload in [
            r#"{"id":"b"}"#,
            r#"{"id":"b","uid":0}"#,
            r#"{"id":"b","uid":-3}"#,
            r#"{"id":"b","uid":1.5}"#,
            r#"{"id":"b","uid":"1001"}"#,
            r#"{"id":"b","uid":5,"nickname":9}"#,
        ] {
            let err = parse("block-user", payload).unwrap_err();
            assert!(
                matches!(&err, CommandError::BadArgument { id, .. } if id == "b"),
                "{payload}: {err:?}"
            );
        }
        let err = parse("unblock-user", r#"{"id":"u"}"#).unwrap_err();
        assert!(matches!(err, CommandError::BadArgument { .. }));
    }

    #[test]
    fn download_model_takes_nothing_from_the_caller() {
        // The model comes from the verified metadata (N-5): the command carries no url, version or hash.
        assert_eq!(
            parse("download-model", r#"{"id":"d"}"#).map(|r| r.command),
            Ok(Command::DownloadModel)
        );
        // A caller from before N-5 still sends them: they are ignored, not refused.
        assert_eq!(
            parse(
                "download-model",
                r#"{"id":"d","url":"http://127.0.0.1:9/model.gguf","version":"m2","sha256":"ab"}"#
            )
            .map(|r| r.command),
            Ok(Command::DownloadModel)
        );
    }

    #[test]
    fn the_download_result_is_an_event_the_app_publishes_itself() {
        assert!(!EVENT_NAMES.contains(&DOWNLOAD_RESULT_EVENT));
        assert_eq!(
            event_topic(DOWNLOAD_RESULT_EVENT),
            "rs/app/event/download-result"
        );
    }

    #[test]
    fn start_translator_parses() {
        assert_eq!(
            parse("start-translator", r#"{"id":"t"}"#).map(|r| r.command),
            Ok(Command::StartTranslator)
        );
    }

    #[test]
    fn the_popup_commands_parse() {
        use resonance_types::{PopupKind, WindowRect};
        assert_eq!(
            parse("open-popup", r#"{"id":"p","kind":"cheatsheet"}"#).map(|r| r.command),
            Ok(Command::OpenPopup {
                kind: PopupKind::CheatSheet
            })
        );
        assert_eq!(
            parse("hide-popup", r#"{"id":"p","kind":"favorites"}"#).map(|r| r.command),
            Ok(Command::HidePopup {
                kind: PopupKind::Favorites
            })
        );
        assert_eq!(
            parse(
                "place-popup",
                r#"{"id":"p","kind":"favorites","x":-20,"y":40,"width":500,"height":640}"#
            )
            .map(|r| r.command),
            Ok(Command::PlacePopup {
                kind: PopupKind::Favorites,
                rect: WindowRect {
                    x: -20,
                    y: 40,
                    width: 500,
                    height: 640
                }
            })
        );
        assert_eq!(
            parse("snapshot-popups", r#"{"id":"p"}"#).map(|r| r.command),
            Ok(Command::SnapshotPopups)
        );
        assert_eq!(
            parse("pin-main", r#"{"id":"p","on":true}"#).map(|r| r.command),
            Ok(Command::PinMain { on: true })
        );
    }

    #[test]
    fn the_popup_commands_refuse_what_they_cannot_act_on() {
        for (name, payload) in [
            ("open-popup", r#"{"id":"p"}"#),
            ("open-popup", r#"{"id":"p","kind":"settings"}"#),
            ("open-popup", r#"{"id":"p","kind":3}"#),
            ("hide-popup", r#"{"id":"p","kind":"main"}"#),
            (
                "place-popup",
                r#"{"id":"p","kind":"favorites","x":1,"y":2,"width":0,"height":9}"#,
            ),
            (
                "place-popup",
                r#"{"id":"p","kind":"favorites","x":1,"y":2,"width":9}"#,
            ),
            (
                "place-popup",
                r#"{"id":"p","kind":"favorites","x":1.5,"y":2,"width":9,"height":9}"#,
            ),
            ("pin-main", r#"{"id":"p"}"#),
            ("pin-main", r#"{"id":"p","on":"yes"}"#),
        ] {
            let err = parse(name, payload).unwrap_err();
            assert!(
                matches!(&err, CommandError::BadArgument { id, .. } if id == "p"),
                "{name} {payload}: {err:?}"
            );
        }
    }

    #[test]
    fn the_update_commands_parse() {
        assert_eq!(
            parse("start-update", r#"{"id":"u1"}"#).map(|r| r.command),
            Ok(Command::StartUpdate)
        );
        assert_eq!(
            parse("restart-update", r#"{"id":"u2"}"#),
            Ok(Request {
                id: "u2".into(),
                command: Command::RestartUpdate
            })
        );
    }

    #[test]
    fn the_window_commands_parse() {
        assert_eq!(
            parse(
                "grow-window",
                r#"{"id":"g","min_width":1200,"min_height":900}"#
            )
            .map(|r| r.command),
            Ok(Command::GrowWindow {
                min_width: 1200,
                min_height: 900
            })
        );
        assert_eq!(
            parse("snapshot", r#"{"id":"s"}"#).map(|r| r.command),
            Ok(Command::Snapshot)
        );
        assert_eq!(
            parse("close-window", r#"{"id":"c"}"#).map(|r| r.command),
            Ok(Command::CloseWindow)
        );
    }

    #[test]
    fn grow_window_needs_two_whole_sizes() {
        for payload in [
            r#"{"id":"g"}"#,
            r#"{"id":"g","min_width":1200}"#,
            r#"{"id":"g","min_width":"1200","min_height":900}"#,
            r#"{"id":"g","min_width":-1,"min_height":900}"#,
            r#"{"id":"g","min_width":12.5,"min_height":900}"#,
            r#"{"id":"g","min_width":0,"min_height":900}"#,
            r#"{"id":"g","min_width":99999999999,"min_height":900}"#,
        ] {
            let err = parse("grow-window", payload).unwrap_err();
            assert!(
                matches!(&err, CommandError::BadArgument { id, .. } if id == "g"),
                "{payload}: {err:?}"
            );
        }
    }

    #[test]
    fn an_ack_can_carry_an_answer() {
        let v: Value =
            serde_json::from_str(&ack_data("s", json!({"x": 1, "width": 900}))).expect("json");
        assert_eq!(
            v,
            json!({"id": "s", "ok": true, "data": {"x": 1, "width": 900}})
        );
    }

    #[test]
    fn the_events_the_app_publishes_itself_are_not_ui_events() {
        // They are not emitted to the window, so `listen_any` would never hear them.
        for name in [UPDATE_STATE_EVENT, APP_STARTED_EVENT] {
            assert!(!EVENT_NAMES.contains(&name), "{name}");
            assert!(!name.contains(['/', '+', '#']), "{name}");
        }
    }

    #[test]
    fn a_topic_that_is_not_a_command_is_refused() {
        for topic in [
            "rs/app/event/ping",
            "rs/test/command/",
            "rs/test/command/a/b",
            "rs/test/other/ping",
        ] {
            assert_eq!(
                parse_command(topic, br#"{"id":"x"}"#),
                Err(CommandError::NotACommandTopic(topic.into())),
                "{topic}"
            );
        }
    }

    #[test]
    fn a_payload_without_a_usable_id_is_refused() {
        assert_eq!(parse("ping", "not json"), Err(CommandError::BadPayload));
        assert_eq!(parse("ping", "[1]"), Err(CommandError::BadPayload));
        for payload in [
            "{}",
            r#"{"id":""}"#,
            r#"{"id":5}"#,
            r#"{"id":"a/b"}"#,
            r#"{"id":"a+"}"#,
            r##"{"id":"#"}"##,
        ] {
            assert_eq!(
                parse("ping", payload),
                Err(CommandError::BadId),
                "{payload}"
            );
        }
    }

    #[test]
    fn an_unknown_command_keeps_its_id_for_the_ack() {
        let err = parse("rm-rf", r#"{"id":"z"}"#).unwrap_err();
        assert_eq!(
            err,
            CommandError::Unknown {
                id: "z".into(),
                name: "rm-rf".into()
            }
        );
        assert_eq!(err.id(), Some("z"));
        assert_eq!(CommandError::BadId.id(), None);
    }

    #[test]
    fn replay_chat_needs_a_path() {
        for payload in [
            r#"{"id":"r"}"#,
            r#"{"id":"r","path":""}"#,
            r#"{"id":"r","path":3}"#,
        ] {
            let err = parse("replay-chat", payload).unwrap_err();
            assert!(
                matches!(&err, CommandError::BadArgument { id, .. } if id == "r"),
                "{payload}: {err:?}"
            );
        }
    }

    #[test]
    fn acks_and_errors_are_json() {
        let ok: Value = serde_json::from_str(&ack("a1", Ok(()))).expect("json");
        assert_eq!(ok, json!({"id": "a1", "ok": true}));
        let bad: Value = serde_json::from_str(&ack("a1", Err("no file"))).expect("json");
        assert_eq!(bad, json!({"id": "a1", "ok": false, "error": "no file"}));
        let err: Value =
            serde_json::from_str(&error_message(&CommandError::BadPayload)).expect("json");
        assert_eq!(err, json!({"error": "the payload is not a JSON object"}));
        let with_id: Value = serde_json::from_str(&error_message(&CommandError::Unknown {
            id: "z".into(),
            name: "x".into(),
        }))
        .expect("json");
        assert_eq!(with_id["id"], "z");
    }
}
