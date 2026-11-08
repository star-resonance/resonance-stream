//! The signals that mirror `config.json`: one per `AppConfig` field, named
//! like it. `to_config` builds the value the backend saves and `apply` loads
//! one, each listing every field in a struct literal / pattern without `..`,
//! so a field added to `AppConfig` and forgotten here is a compile error
//! instead of a setting that silently does not save (or load).

use crate::favorites::{clean_tabs, normalize};
use crate::ui_types::{
    default_catch_up_limit, default_favorite_messages, AppConfig, Channel, ComputeMode,
    FavoriteMessage, FavoriteTab, LogLevel, TabSwitchModifier, Theme, Tier, TranslationView,
    ALL_TAB,
};
use leptos::prelude::{signal, GetUntracked, ReadSignal, Set, WriteSignal};
use resonance_types::FavoritesState;
use resonance_types::GuildRelayConfig;
use std::collections::HashMap;

/// A saved font size of this or less is taken for a broken config.
const MIN_FONT_SIZE: u32 = 8;
/// What a font size at or below [`MIN_FONT_SIZE`] loads as.
const DEFAULT_FONT_SIZE: u32 = 14;
/// What an empty tab-switch key loads as.
const DEFAULT_TAB_SWITCH_KEY: &str = "Tab";

#[derive(Copy, Clone, Debug)]
pub struct ConfigSignals {
    pub init_done: ReadSignal<bool>,
    pub set_init_done: WriteSignal<bool>,
    pub use_translation: ReadSignal<bool>,
    pub set_use_translation: WriteSignal<bool>,
    pub translation_view: ReadSignal<TranslationView>,
    pub set_translation_view: WriteSignal<TranslationView>,
    pub compute_mode: ReadSignal<ComputeMode>,
    pub set_compute_mode: WriteSignal<ComputeMode>,
    pub compact_mode: ReadSignal<bool>,
    pub set_compact_mode: WriteSignal<bool>,
    pub always_on_top: ReadSignal<bool>,
    pub set_always_on_top: WriteSignal<bool>,
    pub active_tab: ReadSignal<String>,
    pub set_active_tab: WriteSignal<String>,
    pub custom_tab_filters: ReadSignal<Vec<String>>,
    pub set_custom_tab_filters: WriteSignal<Vec<String>>,
    pub theme: ReadSignal<Theme>,
    pub set_theme: WriteSignal<Theme>,
    pub overlay_opacity: ReadSignal<f32>,
    pub set_overlay_opacity: WriteSignal<f32>,
    pub debug_mode: ReadSignal<bool>,
    pub set_debug_mode: WriteSignal<bool>,
    pub log_level: ReadSignal<LogLevel>,
    pub set_log_level: WriteSignal<LogLevel>,
    pub tier: ReadSignal<Tier>,
    pub set_tier: WriteSignal<Tier>,
    pub translation_catch_up_limit: ReadSignal<usize>,
    pub set_translation_catch_up_limit: WriteSignal<usize>,
    pub hide_original_in_compact: ReadSignal<bool>,
    pub set_hide_original_in_compact: WriteSignal<bool>,
    pub network_interface: ReadSignal<String>,
    pub set_network_interface: WriteSignal<String>,
    pub drag_to_scroll: ReadSignal<bool>,
    pub set_drag_to_scroll: WriteSignal<bool>,
    pub alert_keywords: ReadSignal<Vec<String>>,
    pub set_alert_keywords: WriteSignal<Vec<String>>,
    pub alert_volume: ReadSignal<f32>,
    pub set_alert_volume: WriteSignal<f32>,
    pub emphasis_keywords: ReadSignal<Vec<String>>,
    pub set_emphasis_keywords: WriteSignal<Vec<String>>,
    pub use_relative_time: ReadSignal<bool>,
    pub set_use_relative_time: WriteSignal<bool>,
    pub font_size: ReadSignal<u32>,
    pub set_font_size: WriteSignal<u32>,
    pub hide_blocked_messages: ReadSignal<bool>,
    pub set_hide_blocked_messages: WriteSignal<bool>,
    pub blocked_users: ReadSignal<HashMap<u64, String>>,
    pub set_blocked_users: WriteSignal<HashMap<u64, String>>,
    pub min_sender_level: ReadSignal<u64>,
    pub set_min_sender_level: WriteSignal<u64>,
    pub auto_sync_latest_dict: ReadSignal<bool>,
    pub set_auto_sync_latest_dict: WriteSignal<bool>,
    pub tab_switch_modifier: ReadSignal<TabSwitchModifier>,
    pub set_tab_switch_modifier: WriteSignal<TabSwitchModifier>,
    pub tab_switch_key: ReadSignal<String>,
    pub set_tab_switch_key: WriteSignal<String>,
    pub tab_limits: ReadSignal<HashMap<String, usize>>,
    pub set_tab_limits: WriteSignal<HashMap<String, usize>>,
    pub archive_ignored_channels: ReadSignal<Vec<String>>,
    pub set_archive_ignored_channels: WriteSignal<Vec<String>>,
    pub message_spacing: ReadSignal<u32>,
    pub set_message_spacing: WriteSignal<u32>,
    pub favorite_messages: ReadSignal<Vec<FavoriteMessage>>,
    pub set_favorite_messages: WriteSignal<Vec<FavoriteMessage>>,
    pub favorite_tabs: ReadSignal<Vec<FavoriteTab>>,
    pub set_favorite_tabs: WriteSignal<Vec<FavoriteTab>>,
    pub chat_log_retention_days: ReadSignal<u32>,
    pub set_chat_log_retention_days: WriteSignal<u32>,
    pub raw_capture: ReadSignal<bool>,
    pub set_raw_capture: WriteSignal<bool>,
    pub guild_relay: ReadSignal<GuildRelayConfig>,
    pub set_guild_relay: WriteSignal<GuildRelayConfig>,
}

impl ConfigSignals {
    /// Every signal with its pre-config default; `apply` overwrites them at start-up.
    pub fn new() -> Self {
        let (init_done, set_init_done) = signal::<bool>(false);
        let (use_translation, set_use_translation) = signal::<bool>(false);
        let (translation_view, set_translation_view) =
            signal::<TranslationView>(TranslationView::default());
        let (compute_mode, set_compute_mode) = signal::<ComputeMode>(ComputeMode::default());
        let (compact_mode, set_compact_mode) = signal::<bool>(false);
        let (always_on_top, set_always_on_top) = signal::<bool>(false);
        let (active_tab, set_active_tab) = signal::<String>(ALL_TAB.to_string());
        let (custom_tab_filters, set_custom_tab_filters) =
            signal::<Vec<String>>(Channel::ALL.map(|c| c.as_str().to_string()).to_vec());
        let (theme, set_theme) = signal::<Theme>(Theme::default());
        let (overlay_opacity, set_overlay_opacity) = signal::<f32>(0.85);
        let (debug_mode, set_debug_mode) = signal::<bool>(false);
        let (log_level, set_log_level) = signal::<LogLevel>(LogLevel::default());
        let (tier, set_tier) = signal::<Tier>(Tier::default());
        let (translation_catch_up_limit, set_translation_catch_up_limit) =
            signal::<usize>(default_catch_up_limit());
        let (hide_original_in_compact, set_hide_original_in_compact) = signal::<bool>(false);
        let (network_interface, set_network_interface) = signal::<String>(String::new());
        let (drag_to_scroll, set_drag_to_scroll) = signal::<bool>(false);
        let (alert_keywords, set_alert_keywords) = signal::<Vec<String>>(Vec::new());
        let (alert_volume, set_alert_volume) = signal::<f32>(0.5);
        let (emphasis_keywords, set_emphasis_keywords) = signal::<Vec<String>>(Vec::new());
        let (use_relative_time, set_use_relative_time) = signal::<bool>(false);
        let (font_size, set_font_size) = signal::<u32>(DEFAULT_FONT_SIZE);
        let (hide_blocked_messages, set_hide_blocked_messages) = signal::<bool>(false);
        let (blocked_users, set_blocked_users) = signal::<HashMap<u64, String>>(HashMap::new());
        let (min_sender_level, set_min_sender_level) = signal::<u64>(1);
        let (auto_sync_latest_dict, set_auto_sync_latest_dict) = signal::<bool>(false);
        let (tab_switch_modifier, set_tab_switch_modifier) =
            signal::<TabSwitchModifier>(TabSwitchModifier::default());
        let (tab_switch_key, set_tab_switch_key) =
            signal::<String>(DEFAULT_TAB_SWITCH_KEY.to_string());
        let (tab_limits, set_tab_limits) = signal::<HashMap<String, usize>>(HashMap::new());
        let (archive_ignored_channels, set_archive_ignored_channels) =
            signal::<Vec<String>>(vec![Channel::World.as_str().to_string()]);
        let (message_spacing, set_message_spacing) = signal::<u32>(4);
        let (favorite_messages, set_favorite_messages) =
            signal::<Vec<FavoriteMessage>>(default_favorite_messages());
        let (favorite_tabs, set_favorite_tabs) = signal::<Vec<FavoriteTab>>(Vec::new());
        let (chat_log_retention_days, set_chat_log_retention_days) = signal::<u32>(0);
        let (raw_capture, set_raw_capture) = signal::<bool>(false);
        let (guild_relay, set_guild_relay) = signal(GuildRelayConfig::default());
        ConfigSignals {
            init_done,
            set_init_done,
            use_translation,
            set_use_translation,
            translation_view,
            set_translation_view,
            compute_mode,
            set_compute_mode,
            compact_mode,
            set_compact_mode,
            always_on_top,
            set_always_on_top,
            active_tab,
            set_active_tab,
            custom_tab_filters,
            set_custom_tab_filters,
            theme,
            set_theme,
            overlay_opacity,
            set_overlay_opacity,
            debug_mode,
            set_debug_mode,
            log_level,
            set_log_level,
            tier,
            set_tier,
            translation_catch_up_limit,
            set_translation_catch_up_limit,
            hide_original_in_compact,
            set_hide_original_in_compact,
            network_interface,
            set_network_interface,
            drag_to_scroll,
            set_drag_to_scroll,
            alert_keywords,
            set_alert_keywords,
            alert_volume,
            set_alert_volume,
            emphasis_keywords,
            set_emphasis_keywords,
            use_relative_time,
            set_use_relative_time,
            font_size,
            set_font_size,
            hide_blocked_messages,
            set_hide_blocked_messages,
            blocked_users,
            set_blocked_users,
            min_sender_level,
            set_min_sender_level,
            auto_sync_latest_dict,
            set_auto_sync_latest_dict,
            tab_switch_modifier,
            set_tab_switch_modifier,
            tab_switch_key,
            set_tab_switch_key,
            tab_limits,
            set_tab_limits,
            archive_ignored_channels,
            set_archive_ignored_channels,
            message_spacing,
            set_message_spacing,
            favorite_messages,
            set_favorite_messages,
            favorite_tabs,
            set_favorite_tabs,
            chat_log_retention_days,
            set_chat_log_retention_days,
            raw_capture,
            set_raw_capture,
            guild_relay,
            set_guild_relay,
        }
    }

    /// The config as the ui holds it now, for `save_config`.
    pub fn to_config(&self) -> AppConfig {
        AppConfig {
            init_done: self.init_done.get_untracked(),
            use_translation: self.use_translation.get_untracked(),
            translation_view: self.translation_view.get_untracked(),
            compute_mode: self.compute_mode.get_untracked(),
            compact_mode: self.compact_mode.get_untracked(),
            always_on_top: self.always_on_top.get_untracked(),
            active_tab: self.active_tab.get_untracked(),
            custom_tab_filters: self.custom_tab_filters.get_untracked(),
            theme: self.theme.get_untracked(),
            overlay_opacity: self.overlay_opacity.get_untracked(),
            debug_mode: self.debug_mode.get_untracked(),
            log_level: self.log_level.get_untracked(),
            tier: self.tier.get_untracked(),
            translation_catch_up_limit: self.translation_catch_up_limit.get_untracked(),
            hide_original_in_compact: self.hide_original_in_compact.get_untracked(),
            network_interface: self.network_interface.get_untracked(),
            drag_to_scroll: self.drag_to_scroll.get_untracked(),
            alert_keywords: self.alert_keywords.get_untracked(),
            alert_volume: self.alert_volume.get_untracked(),
            emphasis_keywords: self.emphasis_keywords.get_untracked(),
            use_relative_time: self.use_relative_time.get_untracked(),
            font_size: self.font_size.get_untracked(),
            hide_blocked_messages: self.hide_blocked_messages.get_untracked(),
            blocked_users: self.blocked_users.get_untracked(),
            min_sender_level: self.min_sender_level.get_untracked(),
            auto_sync_latest_dict: self.auto_sync_latest_dict.get_untracked(),
            tab_switch_modifier: self.tab_switch_modifier.get_untracked(),
            tab_switch_key: self.tab_switch_key.get_untracked(),
            tab_limits: self.tab_limits.get_untracked(),
            archive_ignored_channels: self.archive_ignored_channels.get_untracked(),
            message_spacing: self.message_spacing.get_untracked(),
            favorite_messages: self.favorite_messages.get_untracked(),
            favorite_tabs: self.favorite_tabs.get_untracked(),
            chat_log_retention_days: self.chat_log_retention_days.get_untracked(),
            raw_capture: self.raw_capture.get_untracked(),
            guild_relay: self.guild_relay.get_untracked(),
        }
    }

    /// The favorites held, as `save_favorites` takes them.
    pub fn favorites_state(&self) -> FavoritesState {
        FavoritesState {
            messages: self.favorite_messages.get_untracked(),
            tabs: self.favorite_tabs.get_untracked(),
        }
    }

    /// Takes the favorites the backend holds -- loaded, or saved by another
    /// window -- cleaned like a loaded config: tab names tidied, a message
    /// whose tab is gone filed under the default tab.
    pub fn apply_favorites(&self, favorites: FavoritesState) {
        let tabs = clean_tabs(favorites.tabs);
        let mut messages = favorites.messages;
        normalize(&mut messages, &tabs);
        self.set_favorite_messages.set(messages);
        self.set_favorite_tabs.set(tabs);
    }

    /// Loads a config read from disk into the signals.
    pub fn apply(&self, config: AppConfig) {
        let AppConfig {
            init_done,
            use_translation,
            translation_view,
            compute_mode,
            compact_mode,
            always_on_top,
            active_tab,
            custom_tab_filters,
            theme,
            overlay_opacity,
            debug_mode,
            log_level,
            tier,
            translation_catch_up_limit,
            hide_original_in_compact,
            network_interface,
            drag_to_scroll,
            alert_keywords,
            alert_volume,
            emphasis_keywords,
            use_relative_time,
            font_size,
            hide_blocked_messages,
            blocked_users,
            min_sender_level,
            auto_sync_latest_dict,
            tab_switch_modifier,
            tab_switch_key,
            tab_limits,
            archive_ignored_channels,
            message_spacing,
            favorite_messages,
            favorite_tabs,
            chat_log_retention_days,
            raw_capture,
            guild_relay,
        } = config;
        self.set_init_done.set(init_done);
        self.set_use_translation.set(use_translation);
        self.set_translation_view.set(translation_view);
        self.set_compute_mode.set(compute_mode);
        self.set_compact_mode.set(compact_mode);
        self.set_always_on_top.set(always_on_top);
        self.set_active_tab.set(active_tab);
        self.set_custom_tab_filters.set(custom_tab_filters);
        self.set_theme.set(theme);
        self.set_overlay_opacity.set(overlay_opacity);
        self.set_debug_mode.set(debug_mode);
        self.set_log_level.set(log_level);
        self.set_tier.set(tier);
        self.set_translation_catch_up_limit
            .set(translation_catch_up_limit);
        self.set_hide_original_in_compact
            .set(hide_original_in_compact);
        self.set_network_interface.set(network_interface);
        self.set_drag_to_scroll.set(drag_to_scroll);
        self.set_alert_keywords.set(alert_keywords);
        self.set_alert_volume.set(alert_volume);
        self.set_emphasis_keywords.set(emphasis_keywords);
        self.set_use_relative_time.set(use_relative_time);
        self.set_font_size.set(if font_size > MIN_FONT_SIZE {
            font_size
        } else {
            DEFAULT_FONT_SIZE
        });
        self.set_hide_blocked_messages.set(hide_blocked_messages);
        self.set_blocked_users.set(blocked_users);
        self.set_min_sender_level.set(min_sender_level);
        self.set_auto_sync_latest_dict.set(auto_sync_latest_dict);
        self.set_tab_switch_modifier.set(tab_switch_modifier);
        self.set_tab_switch_key.set(if tab_switch_key.is_empty() {
            DEFAULT_TAB_SWITCH_KEY.to_string()
        } else {
            tab_switch_key
        });
        // An empty map is a config saved before limits existed: keep the defaults.
        if !tab_limits.is_empty() {
            self.set_tab_limits.set(tab_limits);
        }
        self.set_archive_ignored_channels
            .set(archive_ignored_channels);
        self.set_message_spacing.set(message_spacing);
        self.apply_favorites(FavoritesState {
            messages: favorite_messages,
            tabs: favorite_tabs,
        });
        self.set_chat_log_retention_days
            .set(chat_log_retention_days);
        self.set_raw_capture.set(raw_capture);
        self.set_guild_relay.set(guild_relay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_types::FavoriteMessage;

    /// Every field set to something no default is.
    fn odd_config() -> AppConfig {
        AppConfig {
            init_done: true,
            use_translation: true,
            translation_view: TranslationView::Study,
            compute_mode: ComputeMode::Gpu,
            compact_mode: true,
            always_on_top: true,
            active_tab: "길드".into(),
            custom_tab_filters: vec!["PARTY".into()],
            theme: Theme::Light,
            overlay_opacity: 0.3,
            debug_mode: true,
            log_level: LogLevel::Error,
            tier: Tier::VeryHigh,
            translation_catch_up_limit: 7,
            hide_original_in_compact: true,
            network_interface: "10.0.0.5".into(),
            drag_to_scroll: true,
            alert_keywords: vec!["raid".into()],
            alert_volume: 0.9,
            emphasis_keywords: vec!["boss".into()],
            use_relative_time: true,
            font_size: 20,
            hide_blocked_messages: true,
            blocked_users: HashMap::from([(42, "spam".to_string())]),
            min_sender_level: 33,
            auto_sync_latest_dict: true,
            tab_switch_modifier: TabSwitchModifier::Alt,
            tab_switch_key: "q".into(),
            tab_limits: HashMap::from([("GUILD".to_string(), 12)]),
            archive_ignored_channels: vec!["LOCAL".into(), "PARTY".into()],
            message_spacing: 9,
            favorite_messages: vec![FavoriteMessage {
                text: "hi".into(),
                tab: 2,
                ..Default::default()
            }],
            favorite_tabs: vec![tab(2, "레이드"), tab(5, "던전")],
            chat_log_retention_days: 5,
            raw_capture: true,
            guild_relay: GuildRelayConfig {
                enabled: true,
                broker_url: "https://relay.example".into(),
            },
        }
    }

    fn json(config: &AppConfig) -> serde_json::Value {
        serde_json::to_value(config).unwrap()
    }

    #[test]
    fn every_config_field_survives_a_load_and_a_save() {
        let signals = ConfigSignals::new();
        signals.apply(odd_config());
        assert_eq!(json(&signals.to_config()), json(&odd_config()));
    }

    #[test]
    fn fresh_signals_hold_the_pre_config_defaults() {
        let config = ConfigSignals::new().to_config();
        assert!(!config.init_done && !config.use_translation);
        assert_eq!(config.translation_view, TranslationView::On);
        assert_eq!(config.active_tab, ALL_TAB);
        assert_eq!(
            config.custom_tab_filters,
            ["WORLD", "GUILD", "PARTY", "LOCAL", "BEGINNER"]
        );
        assert_eq!(config.archive_ignored_channels, ["WORLD"]);
        assert_eq!((config.font_size, config.message_spacing), (14, 4));
        assert_eq!((config.overlay_opacity, config.alert_volume), (0.85, 0.5));
        assert_eq!(config.min_sender_level, 1);
        assert_eq!(config.tab_switch_key, "Tab");
        assert!(config.tab_limits.is_empty());
        assert_eq!(config.theme, Theme::Dark);
    }

    fn tab(id: u32, name: &str) -> FavoriteTab {
        FavoriteTab {
            id,
            name: name.into(),
        }
    }

    fn msg(text: &str, tab: u32) -> FavoriteMessage {
        FavoriteMessage {
            text: text.into(),
            tab,
            ..Default::default()
        }
    }

    #[test]
    fn hand_edited_favorite_tabs_load_cleaned_and_orphans_go_to_the_default_tab() {
        let signals = ConfigSignals::new();
        let mut config = odd_config();
        config.favorite_tabs = vec![tab(1, " 레이드 "), tab(2, ""), tab(1, "레이드")];
        config.favorite_messages = vec![msg("a", 1), msg("b", 9)];
        signals.apply(config);
        let held = signals.to_config();
        let tabs: Vec<_> = held
            .favorite_tabs
            .iter()
            .map(|t| (t.id, t.name.as_str()))
            .collect();
        assert_eq!(
            tabs,
            [(1, "레이드"), (2, "탭 2"), (3, "레이드")],
            "names tidied, the repeated id made fresh, no tab lost"
        );
        let filed: Vec<_> = held.favorite_messages.iter().map(|f| f.tab).collect();
        assert_eq!(filed, [1, 0]);
    }

    #[test]
    fn a_favorites_update_from_another_window_is_cleaned_like_a_loaded_config() {
        let signals = ConfigSignals::new();
        let before = signals.to_config();
        signals.apply_favorites(FavoritesState {
            messages: vec![msg("a", 4), msg("b", 9)],
            tabs: vec![tab(4, " 레이드 "), tab(4, "레이드"), tab(6, "")],
        });
        let held = signals.favorites_state();
        let tabs: Vec<_> = held.tabs.iter().map(|t| (t.id, t.name.as_str())).collect();
        assert_eq!(tabs, [(4, "레이드"), (7, "레이드"), (6, "탭 3")]);
        let filed: Vec<_> = held.messages.iter().map(|f| f.tab).collect();
        assert_eq!(filed, [4, 0]);
        // Nothing but the favorites moved.
        let after = signals.to_config();
        assert_eq!(after.overlay_opacity, before.overlay_opacity);
        assert_eq!(after.theme, before.theme);
    }

    #[test]
    fn the_favorites_state_is_what_the_signals_hold() {
        let signals = ConfigSignals::new();
        let held = signals.favorites_state();
        let config = signals.to_config();
        assert_eq!(held.messages, config.favorite_messages);
        assert_eq!(held.tabs, config.favorite_tabs);
    }

    #[test]
    fn fresh_signals_have_no_favorite_tabs() {
        assert!(ConfigSignals::new().to_config().favorite_tabs.is_empty());
    }

    #[test]
    fn a_config_with_no_tab_limits_keeps_the_ones_held() {
        let signals = ConfigSignals::new();
        signals.apply(odd_config());
        let mut empty = odd_config();
        empty.tab_limits.clear();
        signals.apply(empty);
        assert_eq!(signals.to_config().tab_limits.get("GUILD"), Some(&12));
    }

    #[test]
    fn a_broken_font_size_or_an_empty_tab_key_loads_as_the_default() {
        let signals = ConfigSignals::new();
        for size in [0, 1, 8] {
            let mut config = odd_config();
            config.font_size = size;
            config.tab_switch_key = String::new();
            signals.apply(config);
            let got = signals.to_config();
            assert_eq!(got.font_size, 14, "size {size}");
            assert_eq!(got.tab_switch_key, "Tab");
        }
        let mut config = odd_config();
        config.font_size = 9;
        signals.apply(config);
        assert_eq!(signals.to_config().font_size, 9);
    }
}
