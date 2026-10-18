//! Platform-independent logic of Resonance Stream: turning captured TCP
//! payloads into chat messages, and preparing text for / cleaning text from
//! the translation model. No Tauri, no Windows APIs — builds and tests on any OS.

pub mod bridge;
pub mod capture;
pub mod config_migration;
pub mod crash_log;
pub mod download;
pub mod favorites_migration;
pub mod flow;
pub mod furigana;
pub mod history;
pub mod kanji_on;
pub mod log_throttle;
pub mod paste;
pub mod protocol;
pub mod replay;
pub mod server_pins;
pub mod signed_metadata;
pub mod sniffer_net;
pub mod test_env;
pub mod text;
pub mod update_feed;
pub mod update_signature;
pub mod window;
pub mod workers;
