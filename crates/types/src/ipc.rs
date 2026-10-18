//! The contract between the ui and the backend: every Tauri command by name, with its arguments
//! and its reply, and every backend -> ui event name. The ui calls a command through its argument
//! struct (`tauri_bridge::call`), the backend registers the same names in `generate_handler!`
//! (a test reads that list), and both sides name events through [`event`].
//!
//! An argument struct serializes to exactly the object the command's parameters take: Tauri turns
//! a snake_case parameter into a camelCase key, so the fields are renamed the same way. Renaming a
//! field, a command or an event is a protocol change.

use crate::{
    AppConfig, ChatMessage, DictionaryStatus, FavoritesState, FolderStatus, NetworkInterface,
    PopupKind, RubySpan, ServiceStates, SystemMessage, UpdateCheckResult, UpdateTarget, WindowRect,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// A Tauri command: its registered name and what it answers. A command that returns
/// `Result<T, String>` answers `T`; its error arrives as the call's error.
pub trait Command: Serialize {
    const NAME: &'static str;
    type Reply: DeserializeOwned;
}

macro_rules! commands {
    ($(
        $(#[$doc:meta])*
        $ty:ident = $name:literal { $($field:ident : $fty:ty),* $(,)? } -> $reply:ty;
    )*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Serialize, Deserialize)]
            #[serde(rename_all = "camelCase")]
            pub struct $ty { $(pub $field: $fty),* }

            impl Command for $ty {
                const NAME: &'static str = $name;
                type Reply = $reply;
            }
        )*

        /// Every command the backend registers, by name.
        pub const COMMAND_NAMES: &[&str] = &[$($name),*];
    };
}

commands! {
    // Model and AI server files.
    CheckModelStatus = "check_model_status" {} -> FolderStatus;
    DownloadModel = "download_model" {} -> ();
    CheckAiServerStatus = "check_ai_server_status" {} -> FolderStatus;
    DownloadAiServer = "download_ai_server" {} -> ();

    // Capture.
    StartSniffer = "start_sniffer_command" {} -> ();
    RestartSniffer = "restart_sniffer_command" {} -> ();
    EnsureFirewallRule = "ensure_firewall_rule_command" {} -> String;
    GetNetworkInterfaces = "get_network_interfaces" {} -> Vec<NetworkInterface>;
    OpenCapturesFolder = "open_captures_folder" {} -> ();
    BlockUser = "block_user_command" { uid: u64, nickname: String } -> ();
    UnblockUser = "unblock_user_command" { uid: u64 } -> ();

    // Translator.
    LaunchTranslator = "launch_translator" {} -> ();
    AiServerHealthCheck = "ai_server_health_check" {} -> bool;

    // History and the system log.
    GetChatHistory = "get_chat_history" {} -> Vec<ChatMessage>;
    GetSystemHistory = "get_system_history" {} -> Vec<SystemMessage>;
    ClearChatHistory = "clear_chat_history" {} -> ();
    GetServiceStates = "get_service_states" {} -> ServiceStates;
    AnnotateFurigana = "annotate_furigana" { texts: Vec<String> } -> Vec<Vec<RubySpan>>;
    /// A line for the system log from the ui. `level` is a `SystemLogLevel` as it serializes.
    UiSystemMessage = "ui_system_message" { level: String, source: String, message: String } -> ();
    /// The backend reads the fields of `ExportMessage` from each message; the path of the file
    /// written is the answer.
    ExportChatLog = "export_chat_log" { logs: Vec<ChatMessage> } -> String;

    // Updates and the dictionary.
    CheckAllUpdates = "check_all_updates" {} -> UpdateCheckResult;
    IgnoreUpdate = "ignore_update" { target: UpdateTarget, version: String } -> ();
    DownloadAppUpdate = "download_app_update" { download_url: String } -> ();
    CancelAppUpdate = "cancel_app_update" {} -> ();
    RestartToApplyUpdate = "restart_to_apply_update" {} -> ();
    SyncDictionary = "sync_dictionary" {} -> String;
    GetDictVersion = "get_dict_version" {} -> String;
    GetDictionaryStatus = "get_dictionary_status" {} -> DictionaryStatus;
    GetLocalDictionary = "get_local_dictionary" {} -> String;
    SaveLocalDictionary = "save_local_dictionary" { content: String } -> ();

    // Settings.
    LoadConfig = "load_config" {} -> AppConfig;
    SaveConfig = "save_config" { config: AppConfig } -> ();
    SaveFavorites = "save_favorites" { favorites: FavoritesState } -> ();
    UpdateGlobalTabShortcut = "update_global_tab_shortcut" { modifier: String, key: String } -> ();
    OpenAppDataFolder = "open_app_data_folder" {} -> ();
    OpenBrowser = "open_browser" { url: String } -> ();

    // Windows and the tray.
    SetAlwaysOnTop = "set_always_on_top" { on_top: bool } -> ();
    SetClickThrough = "set_click_through" { enabled: bool } -> ();
    OpenPopup = "open_popup" { kind: PopupKind } -> ();
    MinimizeWindow = "minimize_window" {} -> ();
    CloseWindow = "close_window" {} -> ();
    /// Grows the window to at least this size; the answer is the size it had before, if it grew.
    GrowWindow = "grow_window" { min_width: f64, min_height: f64 } -> Option<WindowRect>;
    RestoreWindow = "restore_window" { rect: WindowRect } -> ();
    UpdateTrayMenu = "update_tray_menu" { click_through: bool, always_on_top: bool } -> ();
}

/// Every backend -> ui event, by name. The bridge listens to each ([`ALL`]).
pub mod event {
    pub const CHAT_MESSAGE_UPDATE: &str = "chat-message-update";
    pub const SNIFFER_STATE: &str = "sniffer-state";
    pub const TRANSLATOR_STATE: &str = "translator-state";
    pub const TRANSLATION_EVENT: &str = "translation-event";
    pub const SYSTEM_EVENT: &str = "system-event";
    pub const PACKET_EVENT: &str = "packet-event";
    pub const FAVORITES_CHANGED: &str = "favorites-changed";
    pub const FIREWALL_MISSING: &str = "firewall-missing";
    pub const DOWNLOAD_PROGRESS: &str = "download-progress";
    pub const POPUP_SHOWN: &str = "popup-shown";
    pub const GLOBAL_TAB_SWITCH: &str = "global-tab-switch";
    pub const TRAY_TOGGLE_ALWAYS_ON_TOP: &str = "tray-toggle-always-on-top";
    pub const TRAY_TOGGLE_CLICK_THROUGH: &str = "tray-toggle-click-through";

    /// Every event above.
    pub const ALL: [&str; 13] = [
        CHAT_MESSAGE_UPDATE,
        SNIFFER_STATE,
        TRANSLATOR_STATE,
        TRANSLATION_EVENT,
        SYSTEM_EVENT,
        PACKET_EVENT,
        FAVORITES_CHANGED,
        FIREWALL_MISSING,
        DOWNLOAD_PROGRESS,
        POPUP_SHOWN,
        GLOBAL_TAB_SWITCH,
        TRAY_TOGGLE_ALWAYS_ON_TOP,
        TRAY_TOGGLE_CLICK_THROUGH,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::BTreeSet;

    fn wire<C: Command>(args: &C) -> Value {
        serde_json::to_value(args).unwrap()
    }

    #[test]
    fn a_command_without_arguments_sends_an_empty_object() {
        assert_eq!(wire(&GetChatHistory {}), json!({}));
        assert_eq!(wire(&RestartSniffer {}), json!({}));
    }

    /// The keys the ui sent by hand (`serde_json::json!`) before this module, one per command.
    #[test]
    fn every_argument_struct_sends_the_keys_the_backend_reads() {
        assert_eq!(
            wire(&SetAlwaysOnTop { on_top: true }),
            json!({ "onTop": true })
        );
        assert_eq!(
            wire(&SetClickThrough { enabled: true }),
            json!({ "enabled": true })
        );
        assert_eq!(
            wire(&DownloadAppUpdate {
                download_url: "https://x/y.exe".into()
            }),
            json!({ "downloadUrl": "https://x/y.exe" })
        );
        assert_eq!(
            wire(&IgnoreUpdate {
                target: UpdateTarget::App,
                version: "0.7.0".into()
            }),
            json!({ "target": "app", "version": "0.7.0" })
        );
        assert_eq!(
            wire(&GrowWindow {
                min_width: 800.0,
                min_height: 600.0
            }),
            json!({ "minWidth": 800.0, "minHeight": 600.0 })
        );
        assert_eq!(
            wire(&UpdateTrayMenu {
                click_through: true,
                always_on_top: false
            }),
            json!({ "clickThrough": true, "alwaysOnTop": false })
        );
        assert_eq!(
            wire(&BlockUser {
                uid: 7,
                nickname: "n".into()
            }),
            json!({ "uid": 7, "nickname": "n" })
        );
        assert_eq!(wire(&UnblockUser { uid: 7 }), json!({ "uid": 7 }));
        assert_eq!(
            wire(&UpdateGlobalTabShortcut {
                modifier: "Alt".into(),
                key: "1".into()
            }),
            json!({ "modifier": "Alt", "key": "1" })
        );
        assert_eq!(
            wire(&UiSystemMessage {
                level: "Info".into(),
                source: "UI".into(),
                message: "m".into()
            }),
            json!({ "level": "Info", "source": "UI", "message": "m" })
        );
        assert_eq!(
            wire(&AnnotateFurigana {
                texts: vec!["a".into()]
            }),
            json!({ "texts": ["a"] })
        );
        assert_eq!(
            wire(&SaveLocalDictionary {
                content: "{}".into()
            }),
            json!({ "content": "{}" })
        );
        assert_eq!(
            wire(&OpenBrowser {
                url: "https://x".into()
            }),
            json!({ "url": "https://x" })
        );
        assert_eq!(wire(&ExportChatLog { logs: vec![] }), json!({ "logs": [] }));
        let keys = |v: Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(
            keys(wire(&OpenPopup {
                kind: PopupKind::Favorites
            })),
            ["kind"]
        );
        assert_eq!(
            keys(wire(&RestoreWindow {
                rect: WindowRect::default()
            })),
            ["rect"]
        );
        assert_eq!(
            keys(wire(&SaveConfig {
                config: AppConfig::default()
            })),
            ["config"]
        );
        assert_eq!(
            keys(wire(&SaveFavorites {
                favorites: FavoritesState::default()
            })),
            ["favorites"]
        );
    }

    #[test]
    fn command_names_are_distinct_snake_case() {
        let names: BTreeSet<_> = COMMAND_NAMES.iter().collect();
        assert_eq!(names.len(), COMMAND_NAMES.len(), "a name is listed twice");
        for name in COMMAND_NAMES {
            assert!(!name.is_empty());
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name}"
            );
        }
        assert_eq!(SetAlwaysOnTop::NAME, "set_always_on_top");
        assert_eq!(StartSniffer::NAME, "start_sniffer_command");
    }

    /// The commands `src-tauri/src/lib.rs` hands to `generate_handler!`, read as text: the app
    /// crate builds on Windows only, and this check runs anywhere.
    fn registered_commands() -> BTreeSet<String> {
        let lib = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src-tauri/src/lib.rs"
        ))
        .unwrap();
        let start = lib
            .find("generate_handler![")
            .expect("generate_handler! in lib.rs");
        let body = &lib[start + "generate_handler![".len()..];
        let body = &body[..body.find(']').unwrap()];
        body.lines()
            .map(|line| line.split("//").next().unwrap())
            .flat_map(|line| line.split(','))
            .map(|path| path.trim().rsplit("::").next().unwrap().to_string())
            .filter(|name| !name.is_empty())
            .collect()
    }

    #[test]
    fn the_backend_registers_exactly_the_commands_listed_here() {
        let listed: BTreeSet<String> = COMMAND_NAMES.iter().map(|n| n.to_string()).collect();
        let registered = registered_commands();
        assert_eq!(registered.len(), 44, "{registered:?}");
        assert_eq!(listed, registered);
    }

    #[test]
    fn the_event_names_are_the_ones_on_the_wire() {
        assert_eq!(
            event::ALL,
            [
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
            ]
        );
    }
}
