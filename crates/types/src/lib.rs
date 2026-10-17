//! Types that cross the Tauri boundary: the backend serializes them, the UI
//! deserializes them (and some go back the other way). One definition, so the
//! two sides cannot drift apart. Field names and serde attributes ARE the wire
//! format — renaming one is a protocol change.
//!
//! Dependencies stay minimal (serde, serde_with): this crate compiles to wasm for the UI.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod app_config;
pub use app_config::AppConfig;

// --- Chat and system log ---

/// Names of the tabs that are not a channel, as `config.json` stores them
/// (`active_tab`, the keys of `tab_limits`).
pub const ALL_TAB: &str = "전체";
pub const CUSTOM_TAB: &str = "커스텀";
pub const SYSTEM_TAB: &str = "시스템";

/// What `download_app_update` is rejected with when the user cancels it. The
/// update dialog tells a cancel from a failure by this text.
pub const UPDATE_CANCELLED: &str = "Update download cancelled";

/// A chat channel. On the wire and on disk it is its upper-case name
/// (`"WORLD"`, `"GUILD"`, ...); a name this enum does not know -- an old log, a
/// channel the game has that we do not show separately yet (the beginner
/// channel, code 9) -- reads as [`Channel::World`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    #[default]
    World,
    Local,
    Party,
    Guild,
    Beginner,
}

impl Channel {
    /// Every channel, in the order the tabs and the custom-tab menu list them.
    pub const ALL: [Channel; 5] = [
        Channel::World,
        Channel::Guild,
        Channel::Party,
        Channel::Local,
        Channel::Beginner,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::World => "WORLD",
            Channel::Local => "LOCAL",
            Channel::Party => "PARTY",
            Channel::Guild => "GUILD",
            Channel::Beginner => "BEGINNER",
        }
    }

    /// The channel's tab label (Korean), as `config.json` stores the active tab.
    pub fn label(self) -> &'static str {
        match self {
            Channel::World => "월드",
            Channel::Guild => "길드",
            Channel::Party => "파티",
            Channel::Local => "로컬",
            Channel::Beginner => "초보자",
        }
    }

    /// The channel whose tab label is `label`.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|channel| channel.label() == label)
    }

    /// The channel called `name` (exact, upper case); unknown names are world.
    pub fn from_name(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|channel| channel.as_str() == name)
            .unwrap_or_default()
    }

    /// The channel for the number the game sends: 1 world, 2 local, 3 party,
    /// 4 guild, 9 beginner; `None` for a number the game has no channel for.
    pub fn known_code(code: u64) -> Option<Self> {
        match code {
            1 => Some(Channel::World),
            2 => Some(Channel::Local),
            3 => Some(Channel::Party),
            4 => Some(Channel::Guild),
            9 => Some(Channel::Beginner),
            _ => None,
        }
    }

    /// Like [`known_code`](Self::known_code); a number nobody knows is world.
    pub fn from_code(code: u64) -> Self {
        Self::known_code(code).unwrap_or_default()
    }
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Channel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Channel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_name(&String::deserialize(deserializer)?))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatMessage {
    pub pid: u64,
    pub channel: Channel,
    pub nickname: String,
    pub message: String,
    pub timestamp: u64,
    pub uid: u64,
    pub class_id: u64,
    pub level: u64,
    pub sequence_id: u64,
    #[serde(default)]
    pub is_blocked: bool,
    // --- Translation Support ---
    #[serde(default)]
    pub translated: Option<String>,
    #[serde(default)]
    pub nickname_romaji: Option<String>,
    /// Raw bytes of fields the parser does not understand. Empty unless the
    /// backend runs with unknown-field capture on; omitted from the wire when empty.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub unknown_fields: HashMap<String, Vec<u8>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SystemMessage {
    pub pid: u64,       // Unique ID for Leptos 'For' loop keys
    pub timestamp: u64, // Milliseconds for sorting
    pub level: SystemLogLevel,
    pub source: String,  // "Backend", "Sniffer", "Translator"
    pub message: String, // The actual log text
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TranslationResult {
    pub pid: u64,
    pub translated: String,
}

// --- Service state events ---

/// What the packet sniffer is doing. On the wire it is the variant's name
/// (`"Active"`); a name this enum does not know reads as `Off`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SnifferState {
    Starting,
    Binding,
    Pending,
    Active,
    Error,
    /// Also what any unknown state reads as.
    #[default]
    #[serde(other)]
    Off,
}

impl SnifferState {
    /// What the title-bar badge shows for a transitional state.
    pub fn label(self) -> String {
        match self {
            SnifferState::Starting => "STARTING",
            SnifferState::Binding => "BINDING",
            SnifferState::Pending => "PENDING",
            SnifferState::Active => "ACTIVE",
            SnifferState::Error => "ERROR",
            SnifferState::Off => "OFF",
        }
        .to_string()
    }
}

/// What the translator (llama-server and its worker) is doing; the wire form
/// is the name with spaces (`"Loading Model"`), unknown reads as `Off`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TranslatorState {
    Starting,
    #[serde(rename = "Loading Model")]
    LoadingModel,
    #[serde(rename = "Catching Up")]
    CatchingUp,
    Restarting,
    Active,
    Error,
    /// Also what any unknown state reads as.
    #[default]
    #[serde(other)]
    Off,
}

impl TranslatorState {
    /// What the title-bar badge shows for a transitional state.
    pub fn label(self) -> String {
        match self {
            TranslatorState::Starting => "STARTING",
            TranslatorState::LoadingModel => "LOADING MODEL",
            TranslatorState::CatchingUp => "CATCHING UP",
            TranslatorState::Restarting => "RESTARTING",
            TranslatorState::Active => "ACTIVE",
            TranslatorState::Error => "ERROR",
            TranslatorState::Off => "OFF",
        }
        .to_string()
    }
}

/// Orders state changes: every change of either service gets a larger `seq`,
/// so the UI can drop a snapshot (`get_service_states`) that an event
/// overtook. 0: no order known (a payload from before `seq` existed).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SnifferStatePayload {
    pub state: SnifferState,
    pub message: String, // Context or Error message
    #[serde(default)]
    pub seq: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TranslatorStatePayload {
    pub state: TranslatorState,
    pub message: String,
    #[serde(default)]
    pub seq: u64,
}

impl Default for SnifferStatePayload {
    fn default() -> Self {
        Self {
            state: SnifferState::Off,
            message: String::new(),
            seq: 0,
        }
    }
}

impl Default for TranslatorStatePayload {
    fn default() -> Self {
        Self {
            state: TranslatorState::Off,
            message: String::new(),
            seq: 0,
        }
    }
}

/// The last state each service reported. The backend keeps it so a UI that
/// started listening late (or reloaded) can ask for it instead of waiting
/// for the next event.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ServiceStates {
    pub sniffer: SnifferStatePayload,
    pub translator: TranslatorStatePayload,
    #[serde(skip)]
    last_seq: u64,
}

impl ServiceStates {
    fn next_seq(&mut self) -> u64 {
        self.last_seq += 1;
        self.last_seq
    }

    /// Records a sniffer state; returns the payload to emit.
    pub fn set_sniffer(&mut self, state: SnifferState, message: &str) -> SnifferStatePayload {
        self.sniffer = SnifferStatePayload {
            state,
            message: message.to_string(),
            seq: self.next_seq(),
        };
        self.sniffer.clone()
    }

    /// Records a translator state; returns the payload to emit.
    pub fn set_translator(
        &mut self,
        state: TranslatorState,
        message: &str,
    ) -> TranslatorStatePayload {
        self.translator = TranslatorStatePayload {
            state,
            message: message.to_string(),
            seq: self.next_seq(),
        };
        self.translator.clone()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetworkInterface {
    pub name: String,
    pub ip: String,
}

// --- Settings with a fixed set of values ---

/// A setting that `config.json` stores as a lowercase name. Reads leniently:
/// the case is ignored and a name nobody knows is the default, so one bad
/// value never costs the whole file.
macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $wire:literal),+ $(,)? } default $default:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Every value, in the order a menu lists them.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $wire),+
                }
            }

            /// The value called `name` (any case); unknown names are the default.
            pub fn from_name(name: &str) -> Self {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|value| value.as_str().eq_ignore_ascii_case(name))
                    .unwrap_or_default()
            }

            /// Upper-case text for a button.
            pub fn label(self) -> String {
                self.as_str().to_uppercase()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                $name::$default
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Ok(Self::from_name(&String::deserialize(deserializer)?))
            }
        }
    };
}

string_enum! {
    /// Where the translator runs.
    ComputeMode { Cpu => "cpu", Gpu => "gpu" } default Cpu
}

string_enum! {
    /// How much of the model goes to the GPU (VRAM use); only with `Gpu`.
    Tier { Low => "low", Middle => "middle", High => "high", VeryHigh => "very high" } default Middle
}

string_enum! {
    /// The window's colour scheme.
    Theme { Dark => "dark", Light => "light" } default Dark
}

string_enum! {
    /// What a chat row shows of the translation (`config.json`
    /// `translation_view`). `Study` shows the Japanese with furigana and the
    /// translation only while the pointer is over the row. Whether the
    /// translator runs at all is `use_translation`, a separate switch.
    TranslationView { On => "on", Off => "off", Study => "study" } default On
}

string_enum! {
    /// The least severe system-log line the log shows (`config.json` `log_level`).
    LogLevel { Trace => "trace", Debug => "debug", Info => "info", Warn => "warn", Error => "error" } default Info
}

string_enum! {
    /// How severe a system-log line is; the wire form of `SystemMessage.level`.
    SystemLogLevel {
        Trace => "trace",
        Debug => "debug",
        Info => "info",
        Success => "success",
        Warning => "warn",
        Error => "error",
    } default Info
}

string_enum! {
    /// The modifier of the tab-switch shortcut; `NoModifier` (stored as
    /// `"None"`) is the key on its own.
    TabSwitchModifier { Ctrl => "Ctrl", Alt => "Alt", Shift => "Shift", NoModifier => "None" } default Ctrl
}

impl SystemLogLevel {
    /// [`from_name`](Self::from_name), also taking `"warning"` (what the
    /// UI's `ui_system_message` command has always accepted).
    pub fn parse(name: &str) -> Self {
        if name.eq_ignore_ascii_case("warning") {
            SystemLogLevel::Warning
        } else {
            Self::from_name(name)
        }
    }

    /// Where the line ranks for the log-level filter: success counts as info.
    pub fn severity(self) -> LogLevel {
        match self {
            SystemLogLevel::Trace => LogLevel::Trace,
            SystemLogLevel::Debug => LogLevel::Debug,
            SystemLogLevel::Info | SystemLogLevel::Success => LogLevel::Info,
            SystemLogLevel::Warning => LogLevel::Warn,
            SystemLogLevel::Error => LogLevel::Error,
        }
    }
}

impl Theme {
    pub fn toggled(self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }
}

// --- Which dictionary is in use ---

/// Whether the dictionary file on disk is the one that was last synced (the settings view says so
/// next to the sync button). The wire form is the lower-case name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DictionaryState {
    /// The file's SHA-256 is the one recorded at the last sync.
    Same,
    /// It differs: edited here since the sync (the editor saves over the file).
    Modified,
    /// Nothing to compare: never synced, a copy from before the hash was kept, or no file.
    #[default]
    Unknown,
}

/// `get_dictionary_status`: the version last synced, the revision of the signed metadata it came from
/// (0 when unknown) and whether the file still is that one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryStatus {
    pub version: String,
    pub revision: u64,
    pub state: DictionaryState,
}

// --- Favorite messages ---

/// A saved chat line: copied from the favorites panel, or pasted into the
/// game by its global shortcut. `shortcut` is a Tauri accelerator built from
/// `KeyboardEvent.code` ("Ctrl+Shift+Digit1"); empty means none. `note` is
/// a reminder shown under the text (its meaning in Korean); never sent.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteMessage {
    pub text: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub shortcut: String,
    /// Id of the favorites tab it is filed under (`FavoriteTab::id`);
    /// `DEFAULT_FAVORITE_TAB` is the default tab.
    #[serde(default)]
    pub tab: u32,
}

/// The id of the default favorites tab: always there, first, not in
/// `FavoritesState::tabs`, and not removable.
pub const DEFAULT_FAVORITE_TAB: u32 = 0;

/// A user's favorites tab. The id never changes while the tab lives (a message
/// refers to its tab by it), so tabs may share a name, be renamed and be
/// reordered; the order of `FavoritesState::tabs` is the order on screen.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct FavoriteTab {
    pub id: u32,
    pub name: String,
}

/// The favorites as the favorites popup and the main window exchange them
/// (`get_favorites`, `save_favorites`, the `favorites-changed` event): the
/// messages and the tabs. The backend is the one source of truth, so two
/// windows never overwrite each other's settings.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FavoritesState {
    pub messages: Vec<FavoriteMessage>,
    #[serde(default)]
    pub tabs: Vec<FavoriteTab>,
}

/// Missed Japanese messages a translator start translates when the config
/// does not say (`translation_catch_up_limit`, both AppConfigs).
pub fn default_catch_up_limit() -> usize {
    100
}

/// What a fresh config starts with: common Japanese chat lines with their
/// Korean meaning, no shortcuts.
pub fn default_favorite_messages() -> Vec<FavoriteMessage> {
    [
        ("こんにちは！", "안녕하세요"),
        ("おはようございます！", "좋은 아침입니다"),
        ("よろしくお願いします！", "잘 부탁드립니다"),
        ("ありがとうございます！", "감사합니다"),
        ("お疲れ様でした！", "수고하셨습니다"),
        ("すみません！", "죄송합니다"),
        ("またね！", "또 봐요"),
    ]
    .into_iter()
    .map(|(text, note)| FavoriteMessage {
        text: text.to_string(),
        note: note.to_string(),
        ..Default::default()
    })
    .collect()
}

// --- Downloads and updates ---

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FolderStatus {
    pub exists: bool,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProgressPayload {
    pub current_file: String,
    pub percent: u8,
    pub total_percent: u8,
}

/// One entry of the gist's metadata.json (and, for the app, what the release
/// feed announced -- the backend fills `GistMetadata::app` from it).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct VersionInfo {
    pub latest_version: String,
    pub download_url: String,
    pub release_notes: String,
    /// SHA-256 (hex) of the file at `download_url`; downloads are verified
    /// against it. Empty when the gist entry has none.
    #[serde(default)]
    pub sha256: String,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RemoteDictionary {
    pub version: String,
    pub updated_at: String,
    /// SHA-256 of the dictionary file, in the signed metadata: the dictionary has no signature of its
    /// own. Absent in the old gist, which is not signed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GistMetadata {
    /// Which publication of the signed metadata this is (counts up by one each time; the signature
    /// names it). 0 in the old gist, which is not signed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub revision: u64,
    /// The gist's own `app` entry is ignored: the app learns about its updates
    /// from the release feed. The gist keeps it for copies that predate that.
    #[serde(default)]
    pub app: VersionInfo,
    pub model: VersionInfo,
    pub dictionary: RemoteDictionary,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UpdateCheckResult {
    pub app_update_available: bool,
    pub model_update_available: bool,
    pub dict_update_available: bool,
    pub remote_data: GistMetadata,
    /// Why the model and dictionary metadata was refused (a missing or bad signature, an older
    /// revision): when it is set, `remote_data`'s model and dictionary are empty and no model or
    /// dictionary update is offered. `None` for a normal check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_error: Option<String>,
}

impl UpdateCheckResult {
    /// The result of a check whose model / dictionary metadata was refused (`error` says why): the
    /// app's own update (`app_update_available`, `app`: from its own signed feed) is kept, and
    /// nothing the refused metadata said is offered or passed on.
    pub fn refused(app_update_available: bool, app: VersionInfo, error: String) -> Self {
        Self {
            app_update_available,
            model_update_available: false,
            dict_update_available: false,
            remote_data: GistMetadata {
                revision: 0,
                app,
                model: VersionInfo::default(),
                dictionary: RemoteDictionary {
                    version: String::new(),
                    updated_at: String::new(),
                    sha256: String::new(),
                },
            },
            metadata_error: Some(error),
        }
    }
}

// --- Furigana ---

/// A piece of a Japanese line for display: `text` as written, with its
/// hiragana `reading` when it contains kanji. The spans of a line, joined,
/// are the line.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RubySpan {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading: Option<String>,
}

impl RubySpan {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reading: None,
        }
    }

    pub fn with_reading(text: impl Into<String>, reading: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reading: Some(reading.into()),
        }
    }
}

// --- Rules both sides apply ---

/// Whether `text` has Japanese in it (kana or kanji): the backend translates
/// such a line, and the UI marks its original. One definition, so the two
/// cannot disagree.
pub fn contains_japanese(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{3005}'                 // 々 iteration mark
            | '\u{3040}'..='\u{30FF}'  // hiragana, katakana
            | '\u{31F0}'..='\u{31FF}'  // katakana phonetic extensions
            | '\u{3400}'..='\u{4DBF}'  // CJK extension A
            | '\u{4E00}'..='\u{9FFF}'  // CJK unified ideographs
            | '\u{FF66}'..='\u{FF9F}'  // half-width katakana
        )
    })
}

// --- Popup windows ---

/// Every popup window's label starts with this; the capability file allows
/// `popup-*`.
pub const POPUP_LABEL_PREFIX: &str = "popup-";

/// A tool the app opens in a window of its own (not a modal over the chat), so
/// the chat stays visible while it is used. `open_popup` takes one; the same
/// word names the window, and the page in it picks its view from the label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PopupKind {
    /// Class and dungeon names, Japanese and Korean.
    CheatSheet,
    /// Favorite messages: copy, edit, shortcuts.
    Favorites,
}

impl PopupKind {
    pub const ALL: [PopupKind; 2] = [PopupKind::CheatSheet, PopupKind::Favorites];

    /// The window's label.
    pub fn label(self) -> &'static str {
        match self {
            PopupKind::CheatSheet => "popup-cheatsheet",
            PopupKind::Favorites => "popup-favorites",
        }
    }

    /// The kind of the window with this label; `None` for the main window or
    /// anything else.
    pub fn from_label(label: &str) -> Option<PopupKind> {
        PopupKind::ALL.into_iter().find(|k| k.label() == label)
    }

    /// The window's title bar text.
    pub fn title(self) -> &'static str {
        match self {
            PopupKind::CheatSheet => "직업 · 던전 이름",
            PopupKind::Favorites => "자주 쓰는 메시지",
        }
    }

    /// Inner size (width, height) in logical pixels when first opened; the
    /// window-state plugin remembers a resized one.
    pub fn size(self) -> (f64, f64) {
        match self {
            PopupKind::CheatSheet => (420.0, 560.0),
            PopupKind::Favorites => (480.0, 620.0),
        }
    }
}

/// Is this window label a popup's?
pub fn is_popup_label(label: &str) -> bool {
    label.starts_with(POPUP_LABEL_PREFIX)
}

/// How many messages a channel's tab keeps (and the backend keeps and reloads) when the
/// config holds no number for it: WORLD is the busy one, so it keeps fewer. The one place
/// this is decided -- the backend, the chat view, the settings input and a new config all ask here.
pub fn default_channel_limit(channel: Channel) -> usize {
    match channel {
        Channel::World => 500,
        _ => 1000,
    }
}

/// [`default_channel_limit`] for a tab's key in `tab_limits`: the channel's own default, and
/// 1000 for anything that is no channel (it is not read as WORLD).
pub fn default_tab_limit(key: &str) -> usize {
    Channel::ALL
        .into_iter()
        .find(|channel| channel.as_str() == key)
        .map_or(1000, default_channel_limit)
}

/// `tab_limits` of a new config: every channel's default, and the all-tab and custom tab
/// (which have no input of their own) at 1000.
pub fn default_tab_limits() -> HashMap<String, usize> {
    Channel::ALL
        .into_iter()
        .map(|channel| (channel.as_str().to_string(), default_channel_limit(channel)))
        .chain([(ALL_TAB.to_string(), 1000), (CUSTOM_TAB.to_string(), 1000)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_line_from_an_older_version_still_parses() {
        // Regression (W3): a saved line without fields added later (classId,
        // level, sequenceId ...) was skipped, so the history vanished.
        let old =
            r#"{"pid":3,"channel":"WORLD","nickname":"Bob","message":"hi","timestamp":5,"uid":9}"#;
        let msg: ChatMessage = serde_json::from_str(old).expect("old line must parse");
        assert_eq!((msg.pid, msg.uid, msg.message.as_str()), (3, 9, "hi"));
        assert_eq!((msg.class_id, msg.level, msg.sequence_id), (0, 0, 0));
    }

    #[test]
    fn the_dictionary_status_has_a_fixed_wire_form() {
        let status = DictionaryStatus {
            version: "1.0.8".into(),
            revision: 3,
            state: DictionaryState::Modified,
        };
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            r#"{"version":"1.0.8","revision":3,"state":"modified"}"#
        );
        for (state, wire) in [
            (DictionaryState::Same, r#""same""#),
            (DictionaryState::Modified, r#""modified""#),
            (DictionaryState::Unknown, r#""unknown""#),
        ] {
            assert_eq!(serde_json::to_string(&state).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<DictionaryState>(wire).unwrap(),
                state
            );
        }
        // Nothing synced yet: no version, revision 0, and not known to be the published one.
        let none = DictionaryStatus::default();
        assert_eq!((none.version.as_str(), none.revision), ("", 0));
        assert_eq!(none.state, DictionaryState::Unknown);
    }

    #[test]
    fn an_update_check_result_names_why_the_metadata_was_refused_only_when_it_was() {
        // A result from before the field (and a normal one) has none, and says nothing about it on the wire.
        let plain = r#"{"app_update_available":false,"model_update_available":false,"dict_update_available":false,
            "remote_data":{"model":{"latest_version":"1","download_url":"","release_notes":"","sha256":""},
                           "dictionary":{"version":"1","updated_at":""}}}"#;
        let result: UpdateCheckResult =
            serde_json::from_str(plain).expect("an older result parses");
        assert_eq!(result.metadata_error, None);
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("metadata_error"));

        let refused = UpdateCheckResult {
            metadata_error: Some("signature refused".into()),
            ..result
        };
        let text = serde_json::to_string(&refused).unwrap();
        assert!(
            text.contains(r#""metadata_error":"signature refused""#),
            "{text}"
        );
        let back: UpdateCheckResult = serde_json::from_str(&text).unwrap();
        assert_eq!(back.metadata_error.as_deref(), Some("signature refused"));
    }

    #[test]
    fn a_refused_check_keeps_the_app_update_and_offers_no_model_or_dictionary() {
        let announced = VersionInfo {
            latest_version: "0.7.0".into(),
            download_url: "https://example.com/app.exe".into(),
            release_notes: "notes".into(),
            sha256: String::new(),
        };
        let result = UpdateCheckResult::refused(true, announced, "no signature".into());
        // The app's own update comes from its own signed feed and is unaffected.
        assert!(result.app_update_available);
        assert_eq!(result.remote_data.app.latest_version, "0.7.0");
        // Nothing the unverified metadata said is offered or passed on.
        assert!(!result.model_update_available);
        assert!(!result.dict_update_available);
        assert_eq!(result.remote_data.revision, 0);
        assert_eq!(result.remote_data.model.latest_version, "");
        assert_eq!(result.remote_data.model.download_url, "");
        assert_eq!(result.remote_data.model.sha256, "");
        assert_eq!(result.remote_data.dictionary.version, "");
        assert_eq!(result.metadata_error.as_deref(), Some("no signature"));
        // A refusal with no app update to announce.
        let quiet = UpdateCheckResult::refused(false, VersionInfo::default(), "x".into());
        assert!(!quiet.app_update_available);
    }

    #[test]
    fn a_channel_is_its_upper_case_name_on_the_wire() {
        for (channel, name) in [
            (Channel::World, "WORLD"),
            (Channel::Local, "LOCAL"),
            (Channel::Party, "PARTY"),
            (Channel::Guild, "GUILD"),
            (Channel::Beginner, "BEGINNER"),
        ] {
            assert_eq!(serde_json::to_value(channel).unwrap(), name);
            assert_eq!(
                serde_json::from_value::<Channel>(name.into()).unwrap(),
                channel
            );
            assert_eq!(channel.as_str(), name);
            assert_eq!(Channel::from_name(name), channel);
        }
    }

    #[test]
    fn every_channel_has_one_default_limit_and_world_is_the_small_one() {
        // Decided by Kade 2026-10-07: WORLD 500, every other channel 1000.
        assert_eq!(default_channel_limit(Channel::World), 500);
        for channel in [
            Channel::Local,
            Channel::Party,
            Channel::Guild,
            Channel::Beginner,
        ] {
            assert_eq!(default_channel_limit(channel), 1000, "{channel:?}");
        }
    }

    #[test]
    fn a_tab_key_gets_its_channels_default_and_anything_else_1000() {
        assert_eq!(default_tab_limit("WORLD"), 500);
        assert_eq!(default_tab_limit("GUILD"), 1000);
        assert_eq!(default_tab_limit(ALL_TAB), 1000);
        assert_eq!(default_tab_limit("SOMETHING"), 1000); // not read as WORLD
    }

    #[test]
    fn a_new_config_gets_the_default_limits_for_every_tab() {
        let limits = default_tab_limits();
        assert_eq!(limits.len(), Channel::ALL.len() + 2);
        for channel in Channel::ALL {
            assert_eq!(
                limits[channel.as_str()],
                default_channel_limit(channel),
                "{channel:?}"
            );
        }
        assert_eq!(limits[ALL_TAB], 1000);
        assert_eq!(limits[CUSTOM_TAB], 1000);
    }

    #[test]
    fn a_channel_name_nobody_knows_reads_as_world() {
        // Old logs, and channels the game has that the app has no variant for.
        for unknown in ["", "beginner", "world", "9", "길드"] {
            assert_eq!(Channel::from_name(unknown), Channel::World, "{unknown:?}");
            let json = serde_json::Value::String(unknown.into());
            assert_eq!(
                serde_json::from_value::<Channel>(json).unwrap(),
                Channel::World
            );
        }
    }

    #[test]
    fn known_codes_are_the_ones_from_code_does_not_default() {
        // One table: a code the game has no channel for is `None` here and
        // world in `from_code`.
        assert_eq!(Channel::known_code(1), Some(Channel::World));
        assert_eq!(Channel::known_code(9), Some(Channel::Beginner));
        assert_eq!(Channel::known_code(5), None);
        assert_eq!(Channel::from_code(5), Channel::World);
    }

    #[test]
    fn the_game_sends_channels_as_numbers() {
        // One table (was in the parser twice): 2 local, 3 party, 4 guild,
        // 9 beginner, anything else -- 1 is world -- world.
        let got: Vec<_> = [1, 2, 3, 4, 9, 0, 1000].map(Channel::from_code).to_vec();
        assert_eq!(
            got,
            [
                Channel::World,
                Channel::Local,
                Channel::Party,
                Channel::Guild,
                Channel::Beginner,
                Channel::World,
                Channel::World
            ]
        );
    }

    #[test]
    fn a_channel_has_its_korean_tab_label() {
        for (channel, label) in [
            (Channel::World, "월드"),
            (Channel::Guild, "길드"),
            (Channel::Party, "파티"),
            (Channel::Local, "로컬"),
            (Channel::Beginner, "초보자"),
        ] {
            assert_eq!(channel.label(), label);
            assert_eq!(Channel::from_label(label), Some(channel));
        }
        assert_eq!(Channel::from_label("전체"), None);
        assert_eq!(Channel::from_label("WORLD"), None);
    }

    #[test]
    fn the_tabs_that_are_not_channels_keep_their_persisted_names() {
        // config.json stores these (active_tab, tab_limits keys).
        assert_eq!(
            (ALL_TAB, CUSTOM_TAB, SYSTEM_TAB),
            ("전체", "커스텀", "시스템")
        );
    }

    #[test]
    fn every_channel_is_listed_once() {
        assert_eq!(Channel::ALL.len(), 5);
        let names: Vec<_> = Channel::ALL.iter().map(|c| c.as_str()).collect();
        // The order the tabs and the custom-tab menu list them in.
        assert_eq!(names, ["WORLD", "GUILD", "PARTY", "LOCAL", "BEGINNER"]);
        assert_eq!(Channel::default(), Channel::World);
        assert_eq!(Channel::Guild.to_string(), "GUILD");
    }

    #[test]
    fn a_chat_message_keeps_its_channel_string_on_the_wire() {
        let msg = ChatMessage {
            channel: Channel::Guild,
            ..Default::default()
        };
        assert_eq!(serde_json::to_value(&msg).unwrap()["channel"], "GUILD");
        let old = r#"{"pid":1,"channel":"PARTY","message":"x"}"#;
        assert_eq!(
            serde_json::from_str::<ChatMessage>(old).unwrap().channel,
            Channel::Party
        );
        // A line saved without a channel, or with one we never had, is WORLD.
        let none = r#"{"pid":1,"message":"x"}"#;
        assert_eq!(
            serde_json::from_str::<ChatMessage>(none).unwrap().channel,
            Channel::World
        );
    }

    #[test]
    fn empty_unknown_fields_are_left_off_the_wire() {
        let json = serde_json::to_value(ChatMessage {
            pid: 1,
            message: "hi".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(json.get("unknownFields").is_none(), "{json}");
        // ...and the field still deserializes when absent.
        let back: ChatMessage = serde_json::from_value(json).unwrap();
        assert!(back.unknown_fields.is_empty());
    }

    #[test]
    fn version_info_carries_the_sha256() {
        let entry: VersionInfo = serde_json::from_str(
            r#"{"latest_version":"0.5.0","download_url":"https://x/y","release_notes":"","sha256":"ab"}"#,
        )
        .unwrap();
        assert_eq!(entry.sha256, "ab");

        // A missing hash parses as empty (the download then refuses to run),
        // so one bad entry does not break the whole update check.
        let without: VersionInfo = serde_json::from_str(
            r#"{"latest_version":"0.4.0","download_url":"https://x/y","release_notes":""}"#,
        )
        .unwrap();
        assert!(without.sha256.is_empty());
    }

    #[test]
    fn a_gist_without_an_app_entry_still_parses() {
        // The app learns about its own updates from the release feed now; the
        // gist's `app` entry only serves copies that predate that, so it may
        // go once none are left.
        let gist: GistMetadata = serde_json::from_str(
            r#"{"model":{"latest_version":"m1","download_url":"https://x/m","release_notes":""},
                "dictionary":{"version":"d1","updated_at":"2026-10-02"}}"#,
        )
        .unwrap();
        assert!(gist.app.latest_version.is_empty());
        assert_eq!(gist.model.latest_version, "m1");
    }

    #[test]
    fn the_favorites_cross_as_one_camel_case_object() {
        let state = FavoritesState {
            messages: vec![FavoriteMessage {
                text: "hi".into(),
                note: "안녕".into(),
                shortcut: "Alt+F1".into(),
                tab: 3,
            }],
            tabs: vec![FavoriteTab {
                id: 3,
                name: "raid".into(),
            }],
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["messages"][0]["text"], "hi");
        assert_eq!(json["messages"][0]["tab"], 3);
        assert_eq!(json["tabs"][0]["id"], 3);
        assert_eq!(json["tabs"][0]["name"], "raid");
        let back: FavoritesState = serde_json::from_value(json).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn a_favorites_object_missing_its_tabs_still_loads() {
        let state: FavoritesState = serde_json::from_str(r#"{"messages":[]}"#).unwrap();
        assert!(state.messages.is_empty());
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn favorite_defaults_have_no_shortcut_and_shortcut_is_optional() {
        let defaults = default_favorite_messages();
        assert!(!defaults.is_empty());
        assert!(defaults.iter().all(|f| f.shortcut.is_empty()));
        assert!(defaults.iter().all(|f| !f.note.is_empty()));

        // A list saved before notes existed still loads.
        let fav: FavoriteMessage = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
        assert_eq!(fav.text, "hi");
        assert!(fav.note.is_empty());
        assert!(fav.shortcut.is_empty());
        // ... and lands in the default tab.
        assert_eq!(fav.tab, DEFAULT_FAVORITE_TAB);
        assert!(defaults.iter().all(|f| f.tab == DEFAULT_FAVORITE_TAB));
    }

    #[test]
    fn a_favorites_tab_is_stored_under_its_id() {
        let fav = FavoriteMessage {
            text: "hi".into(),
            tab: 2,
            ..Default::default()
        };
        let json = serde_json::to_value(&fav).unwrap();
        assert_eq!(json["tab"], 2);
        assert_eq!(
            serde_json::from_value::<FavoriteMessage>(json).unwrap(),
            fav
        );
    }

    #[test]
    fn the_default_favorites_tab_is_id_zero() {
        assert_eq!(DEFAULT_FAVORITE_TAB, 0);
        assert_eq!(FavoriteMessage::default().tab, DEFAULT_FAVORITE_TAB);
    }

    #[test]
    fn service_states_start_off() {
        let states = ServiceStates::default();
        assert_eq!(states.sniffer.state, SnifferState::Off);
        assert_eq!(states.translator.state, TranslatorState::Off);
        assert_eq!((states.sniffer.seq, states.translator.seq), (0, 0));
    }

    #[test]
    fn service_states_are_their_display_names_on_the_wire() {
        use serde_json::{from_value, json, to_value};
        for (state, name) in [
            (SnifferState::Off, "Off"),
            (SnifferState::Starting, "Starting"),
            (SnifferState::Binding, "Binding"),
            (SnifferState::Pending, "Pending"),
            (SnifferState::Active, "Active"),
            (SnifferState::Error, "Error"),
        ] {
            assert_eq!(to_value(state).unwrap(), name);
            assert_eq!(from_value::<SnifferState>(json!(name)).unwrap(), state);
            assert_eq!(state.label(), name.to_uppercase());
        }
        for (state, name) in [
            (TranslatorState::Off, "Off"),
            (TranslatorState::Starting, "Starting"),
            (TranslatorState::LoadingModel, "Loading Model"),
            (TranslatorState::CatchingUp, "Catching Up"),
            (TranslatorState::Restarting, "Restarting"),
            (TranslatorState::Active, "Active"),
            (TranslatorState::Error, "Error"),
        ] {
            assert_eq!(to_value(state).unwrap(), name);
            assert_eq!(from_value::<TranslatorState>(json!(name)).unwrap(), state);
            assert_eq!(state.label(), name.to_uppercase());
        }
    }

    #[test]
    fn settings_are_their_lowercase_names_on_disk() {
        use serde_json::{from_value, json, to_value};
        for (value, name) in [(ComputeMode::Cpu, "cpu"), (ComputeMode::Gpu, "gpu")] {
            assert_eq!(to_value(value).unwrap(), name);
            assert_eq!(from_value::<ComputeMode>(json!(name)).unwrap(), value);
            assert_eq!(value.label(), name.to_uppercase());
        }
        for (value, name) in [
            (Tier::Low, "low"),
            (Tier::Middle, "middle"),
            (Tier::High, "high"),
            (Tier::VeryHigh, "very high"),
        ] {
            assert_eq!(to_value(value).unwrap(), name);
            assert_eq!(from_value::<Tier>(json!(name)).unwrap(), value);
            assert_eq!(value.label(), name.to_uppercase());
        }
        for (value, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
            assert_eq!(to_value(value).unwrap(), name);
            assert_eq!(from_value::<Theme>(json!(name)).unwrap(), value);
        }
        assert_eq!(
            ComputeMode::ALL.len() + Tier::ALL.len() + Theme::ALL.len(),
            8
        );
    }

    #[test]
    fn the_translation_view_is_its_lowercase_name_and_defaults_to_showing_translations() {
        use serde_json::{from_value, json, to_value};
        for (value, name) in [
            (TranslationView::On, "on"),
            (TranslationView::Off, "off"),
            (TranslationView::Study, "study"),
        ] {
            assert_eq!(to_value(value).unwrap(), name);
            assert_eq!(from_value::<TranslationView>(json!(name)).unwrap(), value);
        }
        // A config from before the setting, or a hand-edited value: today's behaviour.
        assert_eq!(TranslationView::default(), TranslationView::On);
        assert_eq!(
            from_value::<TranslationView>(json!("furigana")).unwrap(),
            TranslationView::On
        );
    }

    #[test]
    fn a_ruby_span_reads_as_plain_text_when_it_has_no_reading() {
        let plain: RubySpan = serde_json::from_str(r#"{"text":"を"}"#).unwrap();
        assert_eq!(plain, RubySpan::plain("を"));
        let with = RubySpan::with_reading("日", "にち");
        assert_eq!(
            serde_json::to_value(&with).unwrap(),
            serde_json::json!({"text": "日", "reading": "にち"})
        );
        // A plain span puts no `reading` key on the wire.
        assert_eq!(
            serde_json::to_value(RubySpan::plain("を")).unwrap(),
            serde_json::json!({"text": "を"})
        );
    }

    #[test]
    fn a_setting_nobody_knows_reads_as_its_default_and_case_does_not_matter() {
        use serde_json::{from_value, json};
        // The defaults are what a fresh config has.
        assert_eq!(ComputeMode::default(), ComputeMode::Cpu);
        assert_eq!(Tier::default(), Tier::Middle);
        assert_eq!(Theme::default(), Theme::Dark);
        assert_eq!(
            from_value::<ComputeMode>(json!("vulkan")).unwrap(),
            ComputeMode::Cpu
        );
        assert_eq!(from_value::<Tier>(json!("extreme")).unwrap(), Tier::Middle);
        assert_eq!(from_value::<Theme>(json!("")).unwrap(), Theme::Dark);
        // The backend always compared these ignoring case.
        assert_eq!(
            from_value::<ComputeMode>(json!("GPU")).unwrap(),
            ComputeMode::Gpu
        );
        assert_eq!(
            from_value::<Tier>(json!("Very High")).unwrap(),
            Tier::VeryHigh
        );
        assert_eq!(from_value::<Theme>(json!("LIGHT")).unwrap(), Theme::Light);
    }

    #[test]
    fn log_levels_are_ordered_by_severity() {
        use LogLevel::*;
        assert!(Trace < Debug && Debug < Info && Info < Warn && Warn < Error);
        assert_eq!(LogLevel::default(), Info);
        assert_eq!(LogLevel::from_name("WARN"), Warn);
        assert_eq!(LogLevel::from_name("warning"), Info); // not a config name
        assert_eq!(serde_json::to_value(Error).unwrap(), "error");
    }

    #[test]
    fn a_system_message_level_is_its_lowercase_name_on_the_wire() {
        use serde_json::{from_value, json, to_value};
        for (level, name) in [
            (SystemLogLevel::Trace, "trace"),
            (SystemLogLevel::Debug, "debug"),
            (SystemLogLevel::Info, "info"),
            (SystemLogLevel::Success, "success"),
            (SystemLogLevel::Warning, "warn"),
            (SystemLogLevel::Error, "error"),
        ] {
            assert_eq!(to_value(level).unwrap(), name);
            assert_eq!(from_value::<SystemLogLevel>(json!(name)).unwrap(), level);
            assert_eq!(level.label(), name.to_uppercase());
        }
        // The UI's command has always taken "warning" too; anything else is info.
        assert_eq!(SystemLogLevel::parse("Warning"), SystemLogLevel::Warning);
        assert_eq!(SystemLogLevel::parse("ERROR"), SystemLogLevel::Error);
        assert_eq!(SystemLogLevel::parse("nonsense"), SystemLogLevel::Info);
        assert_eq!(SystemLogLevel::default(), SystemLogLevel::Info);
    }

    #[test]
    fn a_system_message_ranks_as_a_log_level() {
        // What the log-level filter compares: success counts as info.
        let ranks: Vec<_> = [
            SystemLogLevel::Trace,
            SystemLogLevel::Debug,
            SystemLogLevel::Info,
            SystemLogLevel::Success,
            SystemLogLevel::Warning,
            SystemLogLevel::Error,
        ]
        .map(SystemLogLevel::severity)
        .to_vec();
        use LogLevel::*;
        assert_eq!(ranks, [Trace, Debug, Info, Info, Warn, Error]);
    }

    #[test]
    fn an_old_system_message_still_parses() {
        let old = r#"{"pid":1,"timestamp":2,"level":"warn","source":"Sniffer","message":"m"}"#;
        let msg: SystemMessage = serde_json::from_str(old).unwrap();
        assert_eq!(msg.level, SystemLogLevel::Warning);
        let odd = r#"{"pid":1,"timestamp":2,"level":"loud","source":"s","message":"m"}"#;
        assert_eq!(
            serde_json::from_str::<SystemMessage>(odd).unwrap().level,
            SystemLogLevel::Info
        );
    }

    #[test]
    fn the_tab_switch_modifier_keeps_its_names_and_defaults_to_ctrl() {
        use serde_json::{from_value, json, to_value};
        for (value, name) in [
            (TabSwitchModifier::Ctrl, "Ctrl"),
            (TabSwitchModifier::Alt, "Alt"),
            (TabSwitchModifier::Shift, "Shift"),
            (TabSwitchModifier::NoModifier, "None"),
        ] {
            assert_eq!(to_value(value).unwrap(), name);
            assert_eq!(from_value::<TabSwitchModifier>(json!(name)).unwrap(), value);
        }
        // What a fresh config has -- and what an empty or unknown value reads as.
        assert_eq!(TabSwitchModifier::default(), TabSwitchModifier::Ctrl);
        assert_eq!(TabSwitchModifier::from_name(""), TabSwitchModifier::Ctrl);
        assert_eq!(
            TabSwitchModifier::from_name("Meta"),
            TabSwitchModifier::Ctrl
        );
        assert_eq!(TabSwitchModifier::from_name("alt"), TabSwitchModifier::Alt);
    }

    #[test]
    fn the_theme_toggles() {
        assert_eq!(Theme::Dark.toggled(), Theme::Light);
        assert_eq!(Theme::Light.toggled(), Theme::Dark);
    }

    #[test]
    fn a_state_nobody_knows_reads_as_off() {
        // An older or newer backend, a hand-made payload.
        let sniffer: SnifferStatePayload =
            serde_json::from_str(r#"{"state":"Firewall","message":"m"}"#).unwrap();
        assert_eq!(sniffer.state, SnifferState::Off);
        let translator: TranslatorStatePayload =
            serde_json::from_str(r#"{"state":"","message":"m"}"#).unwrap();
        assert_eq!(translator.state, TranslatorState::Off);
    }

    #[test]
    fn every_state_change_is_newer_than_the_last_across_services() {
        // Regression (A1): the UI had no way to ask for the current state, and
        // could not tell a snapshot from an event that raced it.
        let mut states = ServiceStates::default();
        let a = states.set_translator(TranslatorState::Starting, "init");
        let b = states.set_sniffer(SnifferState::Active, "listening");
        let c = states.set_translator(TranslatorState::Active, "ready");
        assert!(0 < a.seq && a.seq < b.seq && b.seq < c.seq);
        assert_eq!(states.translator, c);
        assert_eq!(states.sniffer, b);
    }

    #[test]
    fn state_payload_carries_seq_and_an_older_one_still_parses() {
        let json =
            serde_json::to_value(ServiceStates::default().set_sniffer(SnifferState::Error, "x"))
                .unwrap();
        assert_eq!(json["seq"], 1);
        let old: TranslatorStatePayload =
            serde_json::from_str(r#"{"state":"Active","message":"m"}"#).unwrap();
        assert_eq!(old.seq, 0);
    }

    #[test]
    fn japanese_is_kana_or_kanji() {
        assert!(contains_japanese("こんにちは"));
        assert!(contains_japanese("カタカナ"));
        assert!(contains_japanese("漢字"));
        assert!(!contains_japanese("hello 123"));
        assert!(!contains_japanese("안녕하세요"));
    }

    #[test]
    fn japanese_the_old_ranges_missed() {
        // B6: half-width katakana, the iteration mark, the phonetic
        // extensions, CJK Extension A and the end of the unified block.
        assert!(contains_japanese("ｱﾘｶﾞﾄｳ"));
        assert!(contains_japanese("々"));
        assert!(contains_japanese("ㇰ"));
        assert!(contains_japanese("㐀"));
        assert!(contains_japanese("\u{9FC0}"));
        // Full-width Latin and half-width Korean are not Japanese.
        assert!(!contains_japanese("ｗｗｗ ＡＢＣ"));
        assert!(!contains_japanese("ﾡ"));
    }

    // --- popup windows ---

    #[test]
    fn a_popup_kind_round_trips_through_its_window_label() {
        for kind in PopupKind::ALL {
            assert_eq!(PopupKind::from_label(kind.label()), Some(kind));
        }
    }

    #[test]
    fn popup_labels_are_distinct_and_start_with_the_popup_prefix() {
        let labels: Vec<_> = PopupKind::ALL.iter().map(|k| k.label()).collect();
        for (i, label) in labels.iter().enumerate() {
            assert!(label.starts_with(POPUP_LABEL_PREFIX), "{label}");
            assert!(!labels[..i].contains(label), "{label} is used twice");
        }
    }

    #[test]
    fn only_popup_labels_are_popups() {
        assert!(is_popup_label("popup-cheatsheet"));
        assert!(!is_popup_label("main"));
        assert!(!is_popup_label(""));
        assert_eq!(PopupKind::from_label("main"), None);
        assert_eq!(PopupKind::from_label("popup-nothing"), None);
    }

    #[test]
    fn a_popup_kind_crosses_as_a_lowercase_word() {
        assert_eq!(
            serde_json::to_string(&PopupKind::CheatSheet).unwrap(),
            "\"cheatsheet\""
        );
        let back: PopupKind = serde_json::from_str("\"favorites\"").unwrap();
        assert_eq!(back, PopupKind::Favorites);
        assert!(serde_json::from_str::<PopupKind>("\"other\"").is_err());
    }

    #[test]
    fn a_popup_has_a_korean_title_and_a_usable_size() {
        for kind in PopupKind::ALL {
            assert!(!kind.title().is_empty());
            let (w, h) = kind.size();
            assert!(w >= 300.0 && h >= 300.0, "{kind:?}: {w}x{h}");
        }
    }

    #[test]
    fn non_empty_unknown_fields_still_cross() {
        let mut msg = ChatMessage::default();
        msg.unknown_fields.insert("chat_40".into(), vec![7]);
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["unknownFields"]["chat_40"][0], 7);
    }
}

// --- Window ---

/// A window's outer position and size in physical pixels. `grow_window` returns
/// the rect it replaced; the UI hands it back to `restore_window`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
