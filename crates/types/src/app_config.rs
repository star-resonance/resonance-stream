//! The settings file's type -- one definition for the backend (which owns the file and writes it)
//! and the ui (which edits it and sends it back). Field names and serde attributes ARE the file
//! format and the wire format. Merged from the app's and the ui's own copies on 2026-10-07
//! (Kade; supersedes the 2026-09-29 decision to keep two).
//!
//! `#[serde(default)]` on the struct: a field missing from the file (an older version, a hand
//! edit) takes its default instead of failing the whole file.

use crate::{
    default_catch_up_limit, default_favorite_messages, Channel, ComputeMode, FavoriteMessage,
    FavoriteTab, LogLevel, TabSwitchModifier, Theme, Tier, TranslationView, ALL_TAB,
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use serde_with::DisplayFromStr;

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct AppConfig {
    pub init_done: bool,
    /// Whether the translator exists at all (the settings switch); what rows
    /// show of it is `translation_view`.
    pub use_translation: bool,
    /// What a chat row shows: translations, only the original, or the study view.
    #[serde(default)]
    pub translation_view: TranslationView,
    pub compute_mode: ComputeMode,
    pub compact_mode: bool,
    pub always_on_top: bool,
    pub active_tab: String,
    pub custom_tab_filters: Vec<String>,
    pub theme: Theme,
    pub overlay_opacity: f32,
    pub debug_mode: bool,
    pub log_level: LogLevel,
    pub tier: Tier,
    /// Japanese messages missed since the app opened that a translator start
    /// translates (the newest ones); 0 turns the catch-up off.
    #[serde(default = "default_catch_up_limit")]
    pub translation_catch_up_limit: usize,
    pub hide_original_in_compact: bool,
    pub network_interface: String,
    pub drag_to_scroll: bool,
    pub alert_keywords: Vec<String>,
    pub alert_volume: f32,
    pub emphasis_keywords: Vec<String>,
    pub use_relative_time: bool,
    pub font_size: u32,
    #[serde(default)]
    pub hide_blocked_messages: bool,
    #[serde_as(as = "std::collections::HashMap<DisplayFromStr, _>")]
    pub blocked_users: std::collections::HashMap<u64, String>,
    #[serde(default)]
    pub min_sender_level: u64,
    #[serde(default)]
    pub auto_sync_latest_dict: bool,
    #[serde(default)]
    pub tab_switch_modifier: TabSwitchModifier,
    #[serde(default)]
    pub tab_switch_key: String, // e.g., "Tab", "ArrowRight", etc.
    /// Messages each tab keeps (keys: channel names, `ALL_TAB`, `CUSTOM_TAB`).
    #[serde(default = "crate::default_tab_limits")]
    pub tab_limits: std::collections::HashMap<String, usize>,
    /// Channels not written to the chat archive.
    #[serde(default = "default_archive_ignored_channels")]
    pub archive_ignored_channels: Vec<String>,
    #[serde(default = "default_spacing")]
    pub message_spacing: u32,
    /// Chat lines to copy or paste by shortcut; see `shortcut.rs`.
    #[serde(default = "default_favorite_messages")]
    pub favorite_messages: Vec<FavoriteMessage>,
    /// The favorites tabs the user made, in display order (the default tab,
    /// id 0, is not listed); each message carries the id of its tab. A file
    /// from before tabs had ids is converted on load (`parse_config`).
    #[serde(default)]
    pub favorite_tabs: Vec<FavoriteTab>,
    /// Days of daily chat logs (chat_logs/) to keep; 0 keeps them all.
    #[serde(default)]
    pub chat_log_retention_days: u32,
    /// Debug: append the game's raw port-5003 packets to `captures/`. Applies live.
    #[serde(default)]
    pub raw_capture: bool,
    /// Sending guild chat to the guild's Discord channel through a broker (#367).
    #[serde(default)]
    pub guild_relay: GuildRelayConfig,
}

/// The guild relay's settings. Off until the member turns it on (decided in #367).
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(default)]
pub struct GuildRelayConfig {
    pub enabled: bool,
    /// The broker's base URL (`https://` only). Empty: no broker, nothing is sent.
    pub broker_url: String,
}

fn default_spacing() -> u32 {
    4
}

fn default_archive_ignored_channels() -> Vec<String> {
    vec![Channel::World.as_str().to_string()]
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            init_done: false,
            use_translation: false,
            translation_view: TranslationView::default(),
            compute_mode: ComputeMode::default(),
            compact_mode: false,
            always_on_top: false,
            active_tab: ALL_TAB.to_string(),
            custom_tab_filters: Channel::ALL.map(|c| c.as_str().to_string()).to_vec(),
            theme: Theme::default(),
            overlay_opacity: 0.85,
            debug_mode: false,
            log_level: LogLevel::default(),
            tier: Tier::default(),
            translation_catch_up_limit: default_catch_up_limit(),
            hide_original_in_compact: false,
            network_interface: "".to_string(),
            drag_to_scroll: false,
            alert_keywords: vec![],
            alert_volume: 0.5,
            emphasis_keywords: vec![],
            use_relative_time: false,
            font_size: 14,
            hide_blocked_messages: false,
            blocked_users: std::collections::HashMap::new(),
            min_sender_level: 1,
            auto_sync_latest_dict: false,
            tab_switch_modifier: TabSwitchModifier::default(),
            tab_switch_key: "Tab".to_string(),
            tab_limits: crate::default_tab_limits(),
            archive_ignored_channels: default_archive_ignored_channels(),
            message_spacing: default_spacing(),
            favorite_messages: default_favorite_messages(),
            favorite_tabs: Vec::new(),
            chat_log_retention_days: 0,
            raw_capture: false,
            guild_relay: GuildRelayConfig::default(),
        }
    }
}

impl AppConfig {
    /// The favorites part of the config, as the windows exchange it.
    pub fn favorites(&self) -> crate::FavoritesState {
        crate::FavoritesState {
            messages: self.favorite_messages.clone(),
            tabs: self.favorite_tabs.clone(),
        }
    }

    pub fn with_favorites(mut self, favorites: crate::FavoritesState) -> Self {
        self.favorite_messages = favorites.messages;
        self.favorite_tabs = favorites.tabs;
        self
    }

    /// This config with the favorites of `stored`. A whole-config save comes
    /// from a window's copy that may not have heard of a favorite another
    /// window just saved; the favorites change only through `save_favorites`.
    pub fn keeping_favorites_of(self, stored: &AppConfig) -> Self {
        self.with_favorites(stored.favorites())
    }

    /// This config with the block list of `stored`: the list changes only through the block and unblock
    /// commands, and a whole-config save must not bring back or drop a sender from an older copy.
    pub fn keeping_block_list_of(mut self, stored: &AppConfig) -> Self {
        self.blocked_users = stored.blocked_users.clone();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field set to a value that is not its default (`testdata/app_config_full.json`). A
    /// field the type does not read or write comes back different from the file.
    const FULL: &str = include_str!("../testdata/app_config_full.json");

    #[test]
    fn the_config_round_trips_every_field() {
        let want: serde_json::Value = serde_json::from_str(FULL).expect("fixture is JSON");
        let config: AppConfig = serde_json::from_str(FULL).expect("fixture reads as the config");
        let got = serde_json::to_value(&config).expect("config serializes");
        assert_eq!(got, want);
    }

    #[test]
    fn a_config_missing_fields_keeps_the_rest() {
        // Regression (N3): one missing field used to reset every setting.
        let config: AppConfig = serde_json::from_str(
            r#"{"init_done": true, "theme": "light", "blocked_users": {"7": "x"}}"#,
        )
        .unwrap();
        assert!(config.init_done);
        assert_eq!(config.theme, Theme::Light);
        assert_eq!(config.blocked_users.get(&7).map(String::as_str), Some("x"));
        assert_eq!(config.font_size, 14); // absent: default
    }

    #[test]
    fn the_translation_view_shows_translations_when_missing_and_round_trips() {
        assert_eq!(AppConfig::default().translation_view, TranslationView::On);
        let old: AppConfig = serde_json::from_str(r#"{"init_done": true}"#).unwrap();
        assert_eq!(old.translation_view, TranslationView::On);
        let config: AppConfig = serde_json::from_str(r#"{"translation_view": "study"}"#).unwrap();
        assert_eq!(config.translation_view, TranslationView::Study);
        let saved = serde_json::to_value(&config).unwrap();
        assert_eq!(saved["translation_view"], "study");
    }

    #[test]
    fn the_guild_relay_is_off_with_no_broker_by_default_and_round_trips() {
        assert_eq!(
            AppConfig::default().guild_relay,
            GuildRelayConfig::default()
        );
        assert!(!AppConfig::default().guild_relay.enabled);
        assert!(AppConfig::default().guild_relay.broker_url.is_empty());
        let old: AppConfig = serde_json::from_str(r#"{"init_done": true}"#).unwrap();
        assert_eq!(old.guild_relay, GuildRelayConfig::default());
        let config: AppConfig = serde_json::from_str(
            r#"{"guild_relay": {"enabled": true, "broker_url": "https://relay.example"}}"#,
        )
        .unwrap();
        assert!(config.guild_relay.enabled);
        assert_eq!(config.guild_relay.broker_url, "https://relay.example");
        let saved = serde_json::to_value(&config).unwrap();
        assert_eq!(saved["guild_relay"]["enabled"], true);
    }

    #[test]
    fn raw_capture_is_off_by_default() {
        assert!(!AppConfig::default().raw_capture);
        let config: AppConfig = serde_json::from_str(r#"{"init_done": true}"#).unwrap();
        assert!(!config.raw_capture);
    }

    #[test]
    fn favorite_tabs_are_empty_when_missing_and_round_trip() {
        assert!(AppConfig::default().favorite_tabs.is_empty());
        let old: AppConfig = serde_json::from_str(r#"{"init_done": true}"#).unwrap();
        assert!(old.favorite_tabs.is_empty());
        let config: AppConfig = serde_json::from_str(
            r#"{"favorite_tabs": [{"id": 3, "name": "레이드"}], "favorite_messages": [{"text": "hi", "tab": 3}]}"#,
        )
        .unwrap();
        assert_eq!(config.favorite_tabs[0].id, 3);
        assert_eq!(config.favorite_tabs[0].name, "레이드");
        assert_eq!(config.favorite_messages[0].tab, 3);
        let saved = serde_json::to_value(&config).unwrap();
        assert_eq!(saved["favorite_tabs"][0]["id"], 3);
        assert_eq!(saved["favorite_tabs"][0]["name"], "레이드");
    }

    #[test]
    fn the_favorites_round_trip_through_the_config() {
        let state = crate::FavoritesState {
            messages: vec![crate::FavoriteMessage {
                text: "hi".into(),
                note: "안녕".into(),
                shortcut: "Alt+F1".into(),
                tab: 1,
            }],
            tabs: vec![FavoriteTab {
                id: 1,
                name: "레이드".into(),
            }],
        };
        let config = AppConfig::default().with_favorites(state.clone());
        assert_eq!(config.favorite_messages, state.messages);
        assert_eq!(config.favorite_tabs, state.tabs);
        assert_eq!(config.favorites(), state);
    }

    #[test]
    fn a_whole_config_save_keeps_the_stored_favorites() {
        // The stored config has a favorite the popup just added; the main
        // window saves its settings from a copy that has not heard of it.
        let stored = AppConfig::default().with_favorites(crate::FavoritesState {
            messages: vec![crate::FavoriteMessage {
                text: "from the popup".into(),
                ..Default::default()
            }],
            tabs: vec![FavoriteTab {
                id: 1,
                name: "레이드".into(),
            }],
        });
        let stale = AppConfig {
            overlay_opacity: 0.5,
            ..AppConfig::default()
        };
        let saved = stale.keeping_favorites_of(&stored);
        assert_eq!(saved.favorites(), stored.favorites());
        assert_eq!(
            saved.overlay_opacity, 0.5,
            "the other settings are the new ones"
        );
    }

    #[test]
    fn a_whole_config_save_keeps_the_stored_block_list() {
        // The block list changes only through the block and unblock commands. A window's
        // copy of it may be older (a block made over the bridge or from another window).
        let stored = AppConfig {
            blocked_users: [(7, "spammer".to_string())].into(),
            ..AppConfig::default()
        };
        let stale = AppConfig {
            overlay_opacity: 0.5,
            blocked_users: [(9, "old".to_string())].into(),
            ..AppConfig::default()
        };
        let saved = stale.keeping_block_list_of(&stored);
        assert_eq!(saved.blocked_users, stored.blocked_users);
        assert_eq!(
            saved.overlay_opacity, 0.5,
            "the other settings are the new ones"
        );
    }

    #[test]
    fn odd_setting_values_still_load() {
        // Hand-edited or from another version: case is ignored, a value
        // nobody knows is the default -- the rest of the file survives.
        let config: AppConfig = serde_json::from_str(
            r#"{"init_done": true, "compute_mode": "GPU", "tier": "extreme", "theme": "neon"}"#,
        )
        .unwrap();
        assert!(config.init_done);
        assert_eq!(config.compute_mode, ComputeMode::Gpu);
        assert_eq!(config.tier, Tier::Middle);
        assert_eq!(config.theme, Theme::Dark);
    }

    #[test]
    fn the_tab_switch_modifier_is_ctrl_when_missing_or_empty() {
        // A config saved before the shortcut existed has no modifier (and no
        // key, so nothing is registered); the ui always showed that as Ctrl.
        // The backend used to read the missing value as "no modifier".
        for json in [
            r#"{"init_done": true}"#,
            r#"{"tab_switch_modifier": ""}"#,
            r#"{"tab_switch_modifier": "Ctrl", "tab_switch_key": "Tab"}"#,
        ] {
            let config: AppConfig = serde_json::from_str(json).unwrap();
            assert_eq!(
                config.tab_switch_modifier,
                TabSwitchModifier::Ctrl,
                "{json}"
            );
        }
        let bare: AppConfig = serde_json::from_str(r#"{"tab_switch_modifier": "None"}"#).unwrap();
        assert_eq!(bare.tab_switch_modifier, TabSwitchModifier::NoModifier);
    }

    #[test]
    fn dictionary_auto_sync_is_off_by_default() {
        assert!(!AppConfig::default().auto_sync_latest_dict);
        // A config file without the field (older versions) reads as off too.
        let mut json = serde_json::to_value(AppConfig::default()).unwrap();
        json.as_object_mut()
            .unwrap()
            .remove("auto_sync_latest_dict");
        let config: AppConfig = serde_json::from_value(json).unwrap();
        assert!(!config.auto_sync_latest_dict);
    }
}
