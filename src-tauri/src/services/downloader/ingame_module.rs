//! The in-game module (`resonance_ingame.dll`) of this app's version: fetched from the GitHub
//! release when the in-game send is turned on, checked, and kept in the app's data folder.
//!
//! Which release: the own tag `v<version>`, else the newest rc whose `SHA256SUMS` line equals
//! the exe's pin (an rc exe has the version of the stable release that will be made from it,
//! so its own tag does not exist yet). `SHA256SUMS` only chooses; trust is the signature and
//! the pin, both checked before the file is moved into place.
//!
//! Everything here blocks: call it from a thread of its own, never from the async runtime.

use super::{fetch, gist};
use resonance_core::download::{
    part_path, publish_dir, sha256_file, SingleRun, CONNECT_TIMEOUT, REMOTE_CALL_TIMEOUT,
};
use resonance_core::ingame_fetch::{
    asset_url, dll_asset, module_dir, module_file, releases_api_url, resolve_tag, sig_asset,
    stale_entries, FetchError, MODULE_LABEL, RELEASE_REPO,
};
use resonance_core::update_signature::verify_with_app_keys;
use resonance_types::ipc::event::DOWNLOAD_PROGRESS;
use resonance_types::{ProgressPayload, UPDATE_CANCELLED};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// `SHA256SUMS` is four short lines.
const SUMS_MAX_BYTES: usize = 64 * 1024;
/// A minisign signature is a few hundred bytes.
const SIGNATURE_MAX_BYTES: usize = 16 * 1024;
/// The release list of 100 releases, notes included.
const LIST_MAX_BYTES: usize = 4 * 1024 * 1024;

/// One module download at a time; turning the setting off cancels it.
static RUN: LazyLock<SingleRun> = LazyLock::new(SingleRun::default);

/// Is a module download running?
pub fn module_running() -> bool {
    RUN.is_running()
}

/// Stops a running module download (the setting was turned off).
pub fn cancel_module() {
    if RUN.is_running() {
        RUN.cancel();
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

/// Does `file` exist with the SHA-256 `pin`?
fn holds_pin(file: &Path, pin: &str) -> bool {
    sha256_file(file).is_ok_and(|hash| hash.eq_ignore_ascii_case(pin))
}

/// Makes sure the module of this version is on disk: `Ok(false)` it already was, `Ok(true)` it
/// was downloaded and checked just now. Removes the folders of other versions afterwards.
/// `pin` is the SHA-256 the exe was built with.
pub fn ensure_module(
    app: &AppHandle,
    pin: &str,
    still_wanted: impl Fn() -> bool,
) -> Result<bool, FetchError> {
    let version = env!("CARGO_PKG_VERSION");
    let data = crate::app_dirs::data(app).map_err(|_| FetchError::Disk)?;
    let file = module_file(&data, version);
    if holds_pin(&file, pin) {
        // Not while a run stages its folder: the clean-up would remove it.
        if let Some(_idle) = RUN.try_start() {
            clean_up(&data, version);
        }
        return Ok(false);
    }

    // One run at a time: a run that ends (or is cancelled) lets the next start.
    let _run = loop {
        if let Some(guard) = RUN.try_start() {
            break guard;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    // The setting may have been turned off while this run waited for the slot.
    if !still_wanted() {
        return Err(FetchError::Cancelled);
    }
    // The run we waited for may have placed it.
    if holds_pin(&file, pin) {
        clean_up(&data, version);
        return Ok(false);
    }

    let result = download(app, &data, version, pin);
    emit_progress(app, 100);
    if result.is_ok() {
        clean_up(&data, version);
    }
    result.map(|()| true)
}

fn download(app: &AppHandle, data: &Path, version: &str, pin: &str) -> Result<(), FetchError> {
    let tag = resolve_tag(
        pin,
        version,
        |tag| {
            let url = asset_url(RELEASE_REPO, tag, "SHA256SUMS");
            let body = block_on(gist::fetch_published(&url, SUMS_MAX_BYTES, "SHA256SUMS"))?;
            Ok(body.and_then(|bytes| String::from_utf8(bytes).ok()))
        },
        || block_on(list_releases()),
    )?;
    log::info!("[in-game module] downloading from release {tag}");

    let dir = module_dir(data, version);
    let staging = part_path(&dir);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|_| FetchError::Disk)?;
    let result = stage(app, &staging, &tag, version, pin)
        .and_then(|()| publish_dir(&staging, &dir).map_err(|_| FetchError::Disk));
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Downloads the module and its signature into `staging` and checks them.
fn stage(
    app: &AppHandle,
    staging: &Path,
    tag: &str,
    version: &str,
    pin: &str,
) -> Result<(), FetchError> {
    let file = staging.join(resonance_core::ingame_send::DLL_NAME);
    let url = asset_url(RELEASE_REPO, tag, &dll_asset(version));
    block_on(fetch::download_file_cancellable(
        app,
        &url,
        &file,
        MODULE_LABEL,
        Some(pin),
        Some(RUN.cancel_flag()),
    ))
    .map_err(|e| classify(&e))?;

    let sig_url = asset_url(RELEASE_REPO, tag, &sig_asset(version));
    let sig = block_on(gist::fetch_published(
        &sig_url,
        SIGNATURE_MAX_BYTES,
        "Module signature",
    ))
    .map_err(|_| FetchError::Network)?
    .ok_or(FetchError::NotPublished)?;
    let sig = String::from_utf8(sig).map_err(|_| FetchError::Rejected)?;
    let bytes = fs::read(&file).map_err(|_| FetchError::Disk)?;
    verify_with_app_keys(&bytes, &sig, version).map_err(|e| {
        log::warn!("[in-game module] signature refused: {e}");
        FetchError::Rejected
    })
}

/// What a failed streaming download means for the user.
fn classify(error: &str) -> FetchError {
    if error == UPDATE_CANCELLED {
        FetchError::Cancelled
    } else if error.starts_with("Download corrupted") || error.starts_with("Download incomplete") {
        FetchError::Rejected
    } else if error.starts_with("Download failed. Server returned: 404") {
        FetchError::NotPublished
    } else {
        log::info!("[in-game module] download failed: {error}");
        FetchError::Network
    }
}

/// The release list, as text.
async fn list_releases() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("ResonanceStream/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REMOTE_CALL_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .get(releases_api_url(RELEASE_REPO))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("the release list returned {}", res.status()));
    }
    let body = gist::read_capped(res, LIST_MAX_BYTES).await?;
    String::from_utf8(body).map_err(|e| e.to_string())
}

fn emit_progress(app: &AppHandle, percent: u8) {
    let _ = app.emit(
        DOWNLOAD_PROGRESS,
        ProgressPayload {
            current_file: MODULE_LABEL.to_string(),
            percent,
            total_percent: percent,
        },
    );
}

/// Removes the module folders of other versions and left-over staging folders. A DLL a game
/// still has loaded cannot be deleted by Windows: that folder stays until the next run.
fn clean_up(data: &Path, own_version: &str) {
    let root = data.join("bin").join("ingame");
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    let names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    for name in stale_entries(&refs, own_version) {
        let dir = root.join(&name);
        let _ = fs::remove_file(dir.join(resonance_core::ingame_send::DLL_NAME));
        if let Err(e) = fs::remove_dir_all(&dir) {
            log::info!("[in-game module] old folder {name} stays: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_stream_is_classified_for_the_user() {
        assert_eq!(classify(UPDATE_CANCELLED), FetchError::Cancelled);
        assert_eq!(
            classify("Download corrupted: SHA-256 is a, expected b"),
            FetchError::Rejected
        );
        assert_eq!(
            classify("Download incomplete: received 1 of 2 bytes"),
            FetchError::Rejected
        );
        assert_eq!(
            classify("Download failed. Server returned: 404 Not Found"),
            FetchError::NotPublished
        );
        assert_eq!(classify("error sending request"), FetchError::Network);
    }

    #[test]
    fn a_file_holds_the_pin_only_when_its_hash_equals_it() {
        let dir = std::env::temp_dir().join(format!("ingame_module_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("m.dll");
        fs::write(&file, b"hello").unwrap();
        let hash = sha256_file(&file).unwrap();
        assert!(holds_pin(&file, &hash));
        assert!(holds_pin(&file, &hash.to_uppercase()));
        assert!(!holds_pin(&file, &"0".repeat(64)));
        assert!(!holds_pin(&dir.join("missing.dll"), &hash));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_up_removes_other_versions_and_staging_folders_only() {
        let data = std::env::temp_dir().join(format!("ingame_clean_test_{}", std::process::id()));
        let root = data.join("bin").join("ingame");
        for name in ["0.6.4", "0.6.5", "0.6.6.part", "keep-me"] {
            fs::create_dir_all(root.join(name)).unwrap();
            fs::write(root.join(name).join("resonance_ingame.dll"), b"x").unwrap();
        }
        clean_up(&data, "0.6.5");
        assert!(!root.join("0.6.4").exists());
        assert!(!root.join("0.6.6.part").exists());
        assert!(root.join("0.6.5").exists());
        assert!(root.join("keep-me").exists());
        let _ = fs::remove_dir_all(&data);
    }
}
