use serde::{Deserialize, Serialize};

// Types shared with the backend live in crates/types -- the settings file's AppConfig too.
pub use resonance_types::{
    contains_japanese, default_catch_up_limit, default_favorite_messages, default_tab_limit,
    AppConfig, Channel, ChatMessage, ComputeMode, DictionaryState, DictionaryStatus,
    FavoriteMessage, FavoriteTab, FolderStatus, GistMetadata, LogLevel, NetworkInterface,
    ProgressPayload, RemoteDictionary, RubySpan, ServiceStates, SnifferState, SnifferStatePayload,
    SystemLogLevel, SystemMessage, TabSwitchModifier, Theme, Tier, TranslationResult,
    TranslationView, TranslatorState, TranslatorStatePayload, UpdateCheckResult, VersionInfo,
    WindowRect, ALL_TAB, CUSTOM_TAB, SYSTEM_TAB,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TauriEvent {
    pub payload: ProgressPayload,
}
