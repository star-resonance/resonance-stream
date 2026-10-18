use crate::config::AppConfig;
use crossbeam_channel::Sender;
use parking_lot::{Mutex, RwLock};
use resonance_core::history::ChatHistory;
use resonance_core::text::Dictionary;

pub use resonance_types::{
    ChatMessage, LogLevel, NetworkInterface, ServiceStates, SnifferState, SnifferStatePayload,
    SystemLogLevel, SystemMessage, TranslationResult, TranslatorState, TranslatorStatePayload,
};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

/// Locks are parking_lot's: they do not poison, so one panicking thread does
/// not make every later command that touches the same state panic too.
pub struct AppState {
    /// The live config. The file on disk is read once at start-up and
    /// written by `save_config`; everything else reads this copy.
    pub config: RwLock<AppConfig>,
    /// Held while a config change is applied (see `save_config`).
    pub config_lock: Mutex<()>,
    /// Held while `metadata.json` is read, changed and written (`modify_metadata`), so two
    /// writers (a sync and a download, say) cannot lose each other's change.
    pub metadata_lock: Mutex<()>,
    pub chat_history: Mutex<ChatHistory>,
    pub system_history: Mutex<VecDeque<SystemMessage>>,
    pub next_pid: AtomicU64,
    pub nickname_cache: Mutex<HashMap<String, String>>,
    /// Swapped whole when the dictionary is synced or edited; the translator
    /// takes a cheap `Arc` clone per job.
    pub dictionary: RwLock<Arc<Dictionary>>,
    /// Who runs: the translator's queue and the sniffer's handle (`services/owner.rs`).
    pub services: crate::services::owner::Services,
    /// Japanese messages of this run still owed a translation; caught up at
    /// each translator start (see `translator::catch_up`).
    pub translation_ledger: Mutex<resonance_core::workers::TranslationLedger>,
    pub data_factory_tx: Mutex<Option<Sender<crate::io::DataFactoryJob>>>,
    /// The last state each service emitted (`get_service_states`): a UI that
    /// was not listening yet still learns it.
    pub service_states: Mutex<ServiceStates>,
    /// The last update check's answers and the app update download (`services/downloader`).
    pub updates: crate::services::downloader::Updates,
    /// What the global shortcuts are bound to (see `shortcut.rs`).
    pub shortcuts: Mutex<crate::shortcut::GlobalShortcuts>,
}
