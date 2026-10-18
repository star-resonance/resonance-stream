use crate::inject_system_message;
use crate::protocol::types::{AppState, SystemLogLevel};
use resonance_core::download::{
    is_newer_version, BodyCap, BodyTooLarge, CONNECT_TIMEOUT, REMOTE_CALL_TIMEOUT,
};
use resonance_core::signed_metadata::{
    accept_dictionary, accept_metadata, dictionary_refusal_line, dictionary_state, refusal_line,
    signature_url, trusted_metadata_keys, MetadataError,
};
use resonance_core::test_env::UpdateState;
use resonance_core::text::Dictionary;
use resonance_core::update_feed::{parse_feed_allowing, UpdateFeed};
use resonance_types::{DictionaryStatus, UpdateTarget};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// The model's and the dictionary's metadata, signed (`metadata.yml` publishes it on the generated
/// `metadata` branch; docs/decisions.md D-28). The gist stays for copies that predate this and is
/// not a fallback: a fallback would defeat the signature.
const METADATA_URL: &str =
    "https://raw.githubusercontent.com/enjay27/resonance-stream/metadata/metadata.json";
/// The update feed of the newest stable release (`release.yml` publishes it);
/// the app learns about its own updates here, not from the gist.
const FEED_URL: &str =
    "https://github.com/enjay27/resonance-stream/releases/latest/download/latest.json";
/// A feed is a few lines of JSON plus release notes.
const FEED_MAX_BYTES: usize = 256 * 1024;
/// The metadata names three versions, a model and its hash. Generous.
const METADATA_MAX_BYTES: usize = 64 * 1024;
/// A minisign signature is a few hundred bytes.
const SIGNATURE_MAX_BYTES: usize = 16 * 1024;
/// The custom dictionary: a list of words. Generous (4 MiB), well above any list a person keeps.
const DICT_MAX_BYTES: usize = 4 * 1024 * 1024;
const DICT_URL: &str =
    "https://raw.githubusercontent.com/enjay27/resonance-stream/metadata/custom_dict.json";

// --- 1. Structs matching the published metadata JSON: shared with the UI ---
pub use resonance_types::{GistMetadata, RemoteDictionary, UpdateCheckResult, VersionInfo};

/// A client for the small calls (feed, metadata, dictionary): it gives up connecting, and gives up
/// on the whole call, instead of waiting on a stalled host while the start-up shows "checking"
/// (review W-8). The downloads have their own, longer rules (`fetch.rs`).
fn remote_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REMOTE_CALL_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

/// The body of `res`, up to `limit` bytes: a longer one is refused at once when the host announces
/// its length, and as it arrives otherwise, so a host that never stops cannot fill the memory.
async fn read_capped(mut res: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if res.content_length().is_some_and(|n| n > limit as u64) {
        return Err(BodyTooLarge { limit }.to_string());
    }
    let mut cap = BodyCap::new(limit);
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
        cap.add(chunk.len()).map_err(|e| e.to_string())?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The release the last update check announced, if there was one.
pub fn announced_update(app: &AppHandle) -> Option<UpdateFeed> {
    app.state::<AppState>().updates.announced.lock().clone()
}

/// GET `url`, up to `limit` bytes. `Ok(None)`: nothing is published there (404). `what` names the
/// file in an error.
async fn fetch_published(url: &str, limit: usize, what: &str) -> Result<Option<Vec<u8>>, String> {
    let res = remote_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(format!("{what} returned: {}", res.status()));
    }
    read_capped(res, limit)
        .await
        .map(Some)
        .map_err(|e| format!("{what}: {e}"))
}

/// The published metadata and its signature, checked. The outer `Err`: they could not be fetched
/// (the network, a server error, no metadata file at all). The inner `Err`: they were fetched and
/// refused (no signature, a bad one, a rollback).
async fn fetch_verified_metadata(
    accepted_revision: u64,
) -> Result<Result<GistMetadata, MetadataError>, String> {
    let url = crate::test_env::metadata_url().unwrap_or(METADATA_URL);
    let body = fetch_published(url, METADATA_MAX_BYTES, "Metadata")
        .await?
        .ok_or_else(|| "Metadata: nothing is published at the metadata address".to_string())?;
    let signature = fetch_published(
        &signature_url(url),
        SIGNATURE_MAX_BYTES,
        "Metadata signature",
    )
    .await?
    .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string());
    let keys = trusted_metadata_keys(crate::test_env::metadata_trust_key());
    Ok(accept_metadata(
        &body,
        signature.as_deref(),
        &keys,
        accepted_revision,
    ))
}

/// A publication that verified: remembered as the highest accepted revision, and kept for
/// `sync_dictionary`. The revision is stored once the metadata's signature has held -- the
/// dictionary is checked later, at the sync, and a mismatch there refuses only the dictionary.
fn accept_verified(app: &AppHandle, verified: &GistMetadata) {
    crate::config::modify_metadata(app, |metadata| {
        let newer = verified.revision > metadata.accepted_revision;
        if newer {
            metadata.accepted_revision = verified.revision;
        }
        newer
    });
    *app.state::<AppState>().updates.verified.lock() = Some(verified.clone());
}

/// The metadata the last check accepted, or a fresh check when there was none (the test bridge
/// syncs without one).
async fn verified_metadata(app: &AppHandle) -> Result<GistMetadata, String> {
    let last = app.state::<AppState>().updates.verified.lock().clone();
    if let Some(verified) = last {
        return Ok(verified);
    }
    verified_metadata_fresh(app).await
}

/// The published metadata, fetched and verified now; accepted like an update check's. A refusal is
/// written to the system log and returned as the error. `download_model` takes the model from it.
pub async fn verified_metadata_fresh(app: &AppHandle) -> Result<GistMetadata, String> {
    let accepted = crate::config::load_metadata(app).accepted_revision;
    match fetch_verified_metadata(accepted).await? {
        Ok(verified) => {
            accept_verified(app, &verified);
            Ok(verified)
        }
        Err(refused) => {
            let line = refusal_line(&refused);
            inject_system_message(app, SystemLogLevel::Error, "Metadata", line.clone());
            Err(line)
        }
    }
}

/// Reads the release feed. `Ok(None)`: no stable release exists yet (404).
async fn fetch_update_feed() -> Result<Option<UpdateFeed>, String> {
    let res = remote_client()?
        .get(crate::test_env::feed_url().unwrap_or(FEED_URL))
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(format!("Update feed returned: {}", res.status()));
    }
    let body = read_capped(res, FEED_MAX_BYTES)
        .await
        .map_err(|e| format!("Update feed: {e}"))?;
    let text = String::from_utf8(body).map_err(|_| "The update feed is not text".to_string())?;
    parse_feed_allowing(&text, crate::test_env::allow_local_http()).map(Some)
}

// --- 2. The Single Unified Fetch Command ---
#[tauri::command]
pub async fn check_all_updates(app: AppHandle) -> Result<UpdateCheckResult, String> {
    if crate::test_env::no_update_check() {
        // Nothing is asked of the network and nothing is announced.
        return Ok(UpdateCheckResult {
            app_update_available: false,
            model_update_available: false,
            dict_update_available: false,
            remote_data: GistMetadata {
                revision: 0,
                app: VersionInfo::default(),
                model: VersionInfo::default(),
                dictionary: RemoteDictionary {
                    version: String::new(),
                    updated_at: String::new(),
                    sha256: String::new(),
                },
            },
            metadata_error: None,
        });
    }
    let metadata = crate::config::load_metadata(&app);
    let current_app_version = app.package_info().version.to_string();

    // The model / dictionary metadata first: fetched, then checked against the signature. A fetch
    // that fails is an error as before; a refusal is not (see below).
    let verdict = fetch_verified_metadata(metadata.accepted_revision).await?;

    // App Check: from the release feed (its own signed channel), not from the metadata. A feed that
    // cannot be read is no update -- never a reason to fail the model / dictionary check, and a
    // refused metadata does not touch it either.
    let mut feed_error = None;
    let feed = match fetch_update_feed().await {
        Ok(feed) => feed,
        Err(e) => {
            log::warn!("[Updater] No app update info: {}", e);
            feed_error = Some(e);
            None
        }
    };
    *app.state::<AppState>().updates.announced.lock() = feed.clone();
    let app_info = feed
        .as_ref()
        .map(|feed| VersionInfo {
            latest_version: feed.version.clone(),
            download_url: feed.url.clone(),
            release_notes: feed.notes.clone(),
            sha256: String::new(),
        })
        .unwrap_or_default();

    // Only a newer version is an update (a stale feed must not offer a
    // downgrade).
    let mut app_update_available = feed
        .as_ref()
        .is_some_and(|feed| is_newer_version(&feed.version, &current_app_version));
    if let Some(ignored) = &metadata.ignored_app_version {
        if ignored == &app_info.latest_version {
            app_update_available = false;
        }
    }

    crate::test_env::report_update(UpdateState::after_check(
        feed_error.as_deref(),
        feed.as_ref()
            .filter(|_| app_update_available)
            .map(|feed| feed.version.as_str()),
    ));

    // A bad or missing signature, or a rollback: no model or dictionary update is offered, what is
    // installed stays, and the system log says why. The UI gets the reason in `metadata_error`.
    let mut remote_data = match verdict {
        Ok(verified) => {
            accept_verified(&app, &verified);
            verified
        }
        Err(refused) => {
            *app.state::<AppState>().updates.verified.lock() = None;
            let line = refusal_line(&refused);
            inject_system_message(&app, SystemLogLevel::Error, "Metadata", line.clone());
            return Ok(UpdateCheckResult::refused(
                app_update_available,
                app_info,
                line,
            ));
        }
    };
    remote_data.app = app_info;

    // Model Check
    let mut model_update_available =
        remote_data.model.latest_version != metadata.current_model_version;
    if let Some(ignored) = &metadata.ignored_model_version {
        if ignored == &remote_data.model.latest_version {
            model_update_available = false;
        }
    }

    // Dictionary Check
    let dict_update_available = remote_data.dictionary.version != metadata.current_dict_version;

    Ok(UpdateCheckResult {
        app_update_available,
        model_update_available,
        dict_update_available,
        remote_data,
        metadata_error: None,
    })
}

/// %APPDATA%/<bundle id>/custom_dict.json
pub fn dictionary_path(app: &AppHandle) -> PathBuf {
    crate::app_dirs::data(app)
        .expect("Failed to resolve AppData directory")
        .join("custom_dict.json")
}

/// Makes `dict` the one the translator uses from its next job on.
fn install_dictionary(app: &AppHandle, dict: Dictionary) {
    if let Some(state) = app.try_state::<crate::AppState>() {
        *state.dictionary.write() = Arc::new(dict);
    }
}

#[tauri::command]
pub async fn sync_dictionary(app: AppHandle) -> Result<String, String> {
    fetch_and_install_dictionary(&app).await
}

/// Fetches the custom dictionary, checks it against the signed metadata (its SHA-256 is named
/// there) and installs it. What is recorded as installed is the verified metadata's version, so a
/// UI (or a caller of the bridge) cannot name another one.
pub async fn fetch_and_install_dictionary(app: &AppHandle) -> Result<String, String> {
    // 1. Resolve Local Path
    let dict_path = dictionary_path(app);

    // 2. What the signed metadata says the dictionary is
    let verified = verified_metadata(app).await?;

    // 3. Fetch from Remote
    let body = fetch_published(
        crate::test_env::dictionary_url().unwrap_or(DICT_URL),
        DICT_MAX_BYTES,
        "Dictionary",
    )
    .await?
    .ok_or_else(|| "Dictionary: nothing is published at the dictionary address".to_string())?;

    // The named file, and one the app can read: otherwise nothing is saved.
    if let Err(refused) = accept_dictionary(&body, &verified) {
        let line = dictionary_refusal_line(&refused);
        inject_system_message(app, SystemLogLevel::Error, "Metadata", line.clone());
        return Err(line);
    }
    let json_content =
        String::from_utf8(body).map_err(|_| "The dictionary is not text".to_string())?;
    let dict = Dictionary::from_json_str(&json_content)
        .map_err(|e| format!("Invalid dictionary received: {}", e))?;

    // 4. Save Locally
    fs::create_dir_all(dict_path.parent().unwrap()).map_err(|e| e.to_string())?;
    resonance_core::download::write_atomic(&dict_path, json_content.as_bytes())
        .map_err(|e| e.to_string())?;
    install_dictionary(app, dict);

    inject_system_message(
        app,
        SystemLogLevel::Success,
        "ModelManager",
        "Dictionary saved to AppData.",
    );

    inject_system_message(
        app,
        SystemLogLevel::Success,
        "Translator",
        "Dictionary successfully synchronized.",
    );
    println!(
        "Dictionary successfully synchronized. version {:?}",
        verified.dictionary.version
    );

    crate::config::modify_metadata(app, |metadata| {
        metadata.current_dict_version = verified.dictionary.version;
        metadata.current_dict_sha256 = verified.dictionary.sha256.trim().to_ascii_lowercase();
        metadata.current_dict_revision = verified.revision;
        true
    });

    Ok("Dictionary updated and reloaded!".to_string())
}

#[tauri::command]
pub fn get_dict_version(app: tauri::AppHandle) -> String {
    let metadata = crate::config::load_metadata(&app);
    metadata.current_dict_version
}

/// Which dictionary is in use: the version and signed revision of the last sync, and whether the
/// file on disk is still that one (the editor saves over it, which makes it `modified`).
#[tauri::command(async)]
pub fn get_dictionary_status(app: tauri::AppHandle) -> DictionaryStatus {
    let metadata = crate::config::load_metadata(&app);
    let file = resonance_core::download::sha256_file(&dictionary_path(&app)).ok();
    DictionaryStatus {
        version: metadata.current_dict_version,
        revision: metadata.current_dict_revision,
        state: dictionary_state(file.as_deref(), &metadata.current_dict_sha256),
    }
}

#[tauri::command]
pub fn get_local_dictionary(app: tauri::AppHandle) -> Result<String, String> {
    let dict_path = dictionary_path(&app);

    if !dict_path.exists() {
        return Ok("{}".to_string());
    }

    std::fs::read_to_string(&dict_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_local_dictionary(app: tauri::AppHandle, content: String) -> Result<(), String> {
    install_local_dictionary(&app, &content)
}

/// Saves the dictionary editor's text as `custom_dict.json` and installs it from the translator's next job
/// on. A text that does not parse is refused and nothing is saved.
pub fn install_local_dictionary(app: &AppHandle, content: &str) -> Result<(), String> {
    let dict = Dictionary::from_json_str(content)?;
    resonance_core::download::write_atomic(&dictionary_path(app), content.as_bytes())
        .map_err(|e| e.to_string())?;
    install_dictionary(app, dict);
    Ok(())
}

/// "Skip this version": the update check stops offering `version` of `target`. Any other target
/// than `app` or `model` is refused when the call is read, before this runs.
#[tauri::command]
pub fn ignore_update(app: AppHandle, target: UpdateTarget, version: String) {
    crate::config::modify_metadata(&app, |metadata| {
        match target {
            UpdateTarget::App => metadata.ignored_app_version = Some(version),
            UpdateTarget::Model => metadata.ignored_model_version = Some(version),
        }
        true
    });
}
