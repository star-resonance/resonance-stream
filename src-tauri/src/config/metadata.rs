use crate::{inject_system_message, SystemLogLevel};
use resonance_core::download::{keep_bad_copy, read_text_retrying, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppMetadata {
    pub current_model_version: String,
    pub current_dict_version: String,
    pub ignored_app_version: Option<String>,
    pub ignored_model_version: Option<String>,
    pub last_update_check: u64,
    /// The highest revision of the signed model / dictionary metadata this copy accepted; a lower
    /// one is a rollback and is refused. 0 before the first (and in a file from before this field).
    #[serde(default)]
    pub accepted_revision: u64,
    /// SHA-256 of the dictionary the last sync installed (the signed metadata names it), and the
    /// revision of the metadata it came from. A local edit does not change them, so a file that no
    /// longer hashes to this one shows as modified. Empty / 0 before the first sync with them.
    #[serde(default)]
    pub current_dict_sha256: String,
    #[serde(default)]
    pub current_dict_revision: u64,
}

impl Default for AppMetadata {
    fn default() -> Self {
        Self {
            current_model_version: "0.0.0".to_string(),
            current_dict_version: "0.0.0".to_string(),
            ignored_app_version: None,
            ignored_model_version: None,
            last_update_check: 0,
            accepted_revision: 0,
            current_dict_sha256: String::new(),
            current_dict_revision: 0,
        }
    }
}

fn get_metadata_path(app: &AppHandle) -> PathBuf {
    let config_dir = crate::app_dirs::config(app).expect("Could not resolve app config dir");
    if !config_dir.exists() {
        let _ = std::fs::create_dir_all(&config_dir);
    }
    config_dir.join("metadata.json")
}

pub fn load_metadata(app: &AppHandle) -> AppMetadata {
    let path = get_metadata_path(app);

    // Not there yet (a first run): the defaults are written. A file that is there but cannot be
    // read or parsed also gives the defaults -- version 0.0.0 asks for a model re-download --
    // so it is kept as `metadata.json.bad` and the user is told.
    let default_meta = AppMetadata::default();
    match read_text_retrying(&path, 3, Duration::from_millis(250)) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(metadata) => return metadata,
            Err(e) => unusable(app, &path, &format!("it could not be parsed ({e})")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            // Unreadable, not damaged: the file is left alone (a save would replace it).
            unusable(app, &path, &format!("it could not be read ({e})"));
            return default_meta;
        }
    }
    save_metadata(app, &default_meta);
    default_meta
}

/// Tells the user that `metadata.json` was not used, and keeps a copy of it.
fn unusable(app: &AppHandle, path: &std::path::Path, why: &str) {
    let kept = match keep_bad_copy(path) {
        Ok(copy) => format!("kept as {}", copy.display()),
        Err(e) => format!("and it could not be copied either ({e})"),
    };
    inject_system_message(
        app,
        SystemLogLevel::Warning,
        "Metadata",
        format!("metadata.json was not used: {why}; the defaults are in use ({kept}). The model may be offered for download again."),
    );
}

fn save_metadata(app: &AppHandle, metadata: &AppMetadata) {
    inject_system_message(
        &app,
        SystemLogLevel::Info,
        "Metadata",
        format!("Metadata saved {:?}", metadata),
    );

    let path = get_metadata_path(app);
    if let Ok(json) = serde_json::to_string_pretty(metadata) {
        if let Err(e) = write_atomic(&path, json.as_bytes()) {
            inject_system_message(
                app,
                SystemLogLevel::Error,
                "Metadata",
                format!("metadata.json was not saved ({e})"),
            );
        }
    }
}

/// Reads `metadata.json`, lets `change` edit it, and writes it when `change` says it changed
/// something -- all under `AppState::metadata_lock`, so concurrent writers do not lose each other's
/// change. The only way the app writes the file (besides the first run's defaults).
pub fn modify_metadata(app: &AppHandle, change: impl FnOnce(&mut AppMetadata) -> bool) {
    let state = app.state::<crate::AppState>();
    let _writing = state.metadata_lock.lock();
    let mut metadata = load_metadata(app);
    if change(&mut metadata) {
        save_metadata(app, &metadata);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_from_before_the_accepted_revision_has_accepted_nothing() {
        let old = r#"{"current_model_version":"1.1.0","current_dict_version":"1.0.6",
            "ignored_app_version":null,"ignored_model_version":null,"last_update_check":0}"#;
        let metadata: AppMetadata = serde_json::from_str(old).expect("an older file parses");
        assert_eq!(metadata.accepted_revision, 0);
        assert_eq!(AppMetadata::default().accepted_revision, 0);
    }

    #[test]
    fn the_accepted_revision_survives_a_save_and_a_load() {
        let metadata = AppMetadata {
            accepted_revision: 7,
            ..AppMetadata::default()
        };
        let text = serde_json::to_string_pretty(&metadata).unwrap();
        let back: AppMetadata = serde_json::from_str(&text).unwrap();
        assert_eq!(back.accepted_revision, 7);
    }

    #[test]
    fn a_file_from_before_the_dictionary_hash_has_none_and_it_survives_a_save() {
        let old = r#"{"current_model_version":"1.1.0","current_dict_version":"1.0.6",
            "ignored_app_version":null,"ignored_model_version":null,"last_update_check":0}"#;
        let metadata: AppMetadata = serde_json::from_str(old).expect("an older file parses");
        assert_eq!(metadata.current_dict_sha256, "");
        assert_eq!(metadata.current_dict_revision, 0);

        let synced = AppMetadata {
            current_dict_sha256: "ab12".into(),
            current_dict_revision: 3,
            ..AppMetadata::default()
        };
        let back: AppMetadata =
            serde_json::from_str(&serde_json::to_string_pretty(&synced).unwrap()).unwrap();
        assert_eq!(
            (
                back.current_dict_sha256.as_str(),
                back.current_dict_revision
            ),
            ("ab12", 3)
        );
    }
}
