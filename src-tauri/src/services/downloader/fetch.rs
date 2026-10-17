//! The one streaming download used by the model, the llama server and the
//! app updater.

use super::ProgressPayload;
use futures_util::StreamExt;
use resonance_core::download::{
    check_download_url_allowing, stall_error, DownloadCheck, ProgressThrottle, StallWatch,
    CONNECT_TIMEOUT, STALL_TIMEOUT,
};
use resonance_types::UPDATE_CANCELLED;
use std::fs;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// How often a wait wakes up to look at the cancel flag.
const CANCEL_TICK: Duration = Duration::from_secs(1);

/// Downloads `url` to `dest`, emitting `download-progress` (labelled
/// `label`) once per whole percent.
///
/// The bytes go to `<dest>.part` first. Only a download that is complete
/// (matches `Content-Length`) and intact (matches `expected_sha256`, when
/// given) is renamed over `dest`, so a failed download never destroys the
/// file that was there before.
///
/// Gives up when the host does not answer (`CONNECT_TIMEOUT`) or no data
/// arrives for `STALL_TIMEOUT`, instead of leaving the progress at 0% forever.
pub async fn download_file(
    app: &AppHandle,
    url: &str,
    dest: &Path,
    label: &str,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    download_file_cancellable(app, url, dest, label, expected_sha256, None).await
}

/// [`download_file`] that also stops, within a second, once `cancel` is set;
/// the error is then `UPDATE_CANCELLED` and no `.part` file is left.
pub async fn download_file_cancellable(
    app: &AppHandle,
    url: &str,
    dest: &Path,
    label: &str,
    expected_sha256: Option<&str>,
    cancel: Option<&AtomicBool>,
) -> Result<(), String> {
    check_download_url_allowing(url, crate::test_env::allow_local_http())?;
    if expected_sha256.is_none() {
        log::warn!(
            "[Download] No SHA-256 published for {}; integrity not verified",
            url
        );
    }

    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut watch = StallWatch::new(STALL_TIMEOUT, Instant::now());

    let res = guarded(client.get(url).send(), &watch, cancel)
        .await?
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!(
            "Download failed. Server returned: {}",
            res.status()
        ));
    }

    let total_size = res.content_length();
    let mut check = DownloadCheck::new(total_size, expected_sha256);
    let mut progress = ProgressThrottle::default();

    let part = part_path(dest);
    let result = async {
        let mut file = fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut stream = res.bytes_stream();
        // The headers just arrived: the body's wait starts now.
        watch.data_arrived(1, Instant::now());
        while let Some(item) = guarded(stream.next(), &watch, cancel).await? {
            let chunk = item.map_err(|e| e.to_string())?;
            watch.data_arrived(chunk.len(), Instant::now());
            file.write_all(&chunk).map_err(|e| e.to_string())?;
            check.update(&chunk);
            if let Some(percent) = progress.update(check.received(), total_size.unwrap_or(0)) {
                let _ = app.emit(
                    resonance_types::ipc::event::DOWNLOAD_PROGRESS,
                    ProgressPayload {
                        current_file: label.to_string(),
                        percent,
                        total_percent: percent,
                    },
                );
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        check.finish()?;
        fs::rename(&part, dest).map_err(|e| e.to_string())
    }
    .await;

    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

/// Waits for `future`, but not past what `watch` allows: `Err` with the stall
/// reason once no data has moved for its limit, or with `UPDATE_CANCELLED`
/// once `cancel` is set (looked at every `CANCEL_TICK`).
async fn guarded<F: Future>(
    future: F,
    watch: &StallWatch,
    cancel: Option<&AtomicBool>,
) -> Result<F::Output, String> {
    let mut future = std::pin::pin!(future);
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(UPDATE_CANCELLED.to_string());
        }
        let now = Instant::now();
        if watch.is_stalled(now) {
            return Err(stall_error(watch.limit()));
        }
        let wait = watch.remaining(now).min(CANCEL_TICK);
        if let Ok(output) = tokio::time::timeout(wait, &mut future).await {
            return Ok(output);
        }
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_core::download::stall_error;
    use resonance_types::UPDATE_CANCELLED;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn run<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    #[test]
    fn a_ready_future_passes_through() {
        let watch = StallWatch::new(Duration::from_secs(30), Instant::now());
        assert_eq!(run(guarded(async { 7 }, &watch, None)), Ok(7));
    }

    #[test]
    fn a_future_that_never_finishes_is_a_stall() {
        let limit = Duration::from_secs(1);
        let watch = StallWatch::new(limit, Instant::now());
        let started = Instant::now();
        let out = run(guarded(std::future::pending::<()>(), &watch, None));
        assert_eq!(out, Err(stall_error(limit)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_cancel_set_beforehand_wins_at_once() {
        let watch = StallWatch::new(Duration::from_secs(30), Instant::now());
        let cancel = AtomicBool::new(true);
        let out = run(guarded(std::future::pending::<()>(), &watch, Some(&cancel)));
        assert_eq!(out, Err(UPDATE_CANCELLED.to_string()));
    }

    #[test]
    fn a_cancel_during_the_wait_ends_it_within_a_tick() {
        let watch = StallWatch::new(Duration::from_secs(30), Instant::now());
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            setter.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let out = run(guarded(
            std::future::pending::<()>(),
            &watch,
            Some(&*cancel),
        ));
        assert_eq!(out, Err(UPDATE_CANCELLED.to_string()));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
