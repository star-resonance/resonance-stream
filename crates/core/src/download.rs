//! Download and update checks, kept free of Tauri and networking so they can
//! be tested on any OS: URL policy, integrity checks, progress throttling
//! and version comparison.

use crate::test_env::is_local_http_url;
use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Only HTTPS downloads: the app runs as Administrator and executes what it
/// downloads (the updater, the llama server), so a plain-HTTP URL could be
/// swapped in transit.
pub fn check_download_url(url: &str) -> Result<(), String> {
    check_download_url_allowing(url, false)
}

/// [`check_download_url`], except that with `allow_local_http` a plain `http://`
/// URL to this machine passes too. Only a test run that points the update feed
/// at a local mock server asks for it; the downloaded exe must still carry a
/// valid signature.
pub fn check_download_url_allowing(url: &str, allow_local_http: bool) -> Result<(), String> {
    let is_https = url
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
    if (is_https && url.len() > 8) || (allow_local_http && is_local_http_url(url)) {
        Ok(())
    } else {
        Err(format!(
            "Refusing to download from a non-HTTPS URL: {:?}",
            url
        ))
    }
}

/// Whether `open_browser` may open `url`: a plain `https` page, with a host, and no space,
/// control character or backslash anywhere. The ui only ever asks for github.com; the app runs
/// as Administrator and the opener would hand `file:`, `ms-msdt:` and the like to Windows, so
/// anything else is refused (review W-9).
pub fn check_open_url(url: &str) -> Result<(), String> {
    let refused = || {
        Err(format!(
            "Refusing to open {:?}: only plain https pages are opened",
            url
        ))
    };
    let is_https = url
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
    if !is_https {
        return refused();
    }
    let rest = &url[8..];
    let bad_char = url
        .chars()
        .any(|c| c.is_ascii_control() || c == ' ' || c == '\\');
    if rest.is_empty() || bad_char || rest.starts_with(['/', '?', '#']) {
        return refused();
    }
    Ok(())
}

/// Checks a finished download: its size against `Content-Length` (when the
/// server sent one) and its SHA-256 against `expected_sha256` (when known).
pub struct DownloadCheck {
    hasher: Sha256,
    received: u64,
    expected_len: Option<u64>,
    expected_sha256: Option<String>,
}

impl DownloadCheck {
    pub fn new(expected_len: Option<u64>, expected_sha256: Option<&str>) -> Self {
        Self {
            hasher: Sha256::new(),
            received: 0,
            expected_len,
            expected_sha256: expected_sha256.map(|h| h.trim().to_ascii_lowercase()),
        }
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
        self.received += chunk.len() as u64;
    }

    pub fn received(&self) -> u64 {
        self.received
    }

    pub fn finish(self) -> Result<(), String> {
        if let Some(expected) = self.expected_len {
            if self.received != expected {
                return Err(format!(
                    "Download incomplete: received {} of {} bytes",
                    self.received, expected
                ));
            }
        }
        if let Some(expected) = self.expected_sha256 {
            let actual = hex(&self.hasher.finalize());
            if actual != expected {
                return Err(format!(
                    "Download corrupted: SHA-256 is {}, expected {}",
                    actual, expected
                ));
            }
        }
        Ok(())
    }
}

/// Turns byte counts into whole percents, reporting only changes, so a
/// multi-GB download emits ~100 progress events instead of one per chunk.
#[derive(Debug, Default)]
pub struct ProgressThrottle {
    last: Option<u8>,
}

impl ProgressThrottle {
    /// `Some(percent)` when it changed since the last report; `None` when it
    /// did not, or when the total size is unknown.
    pub fn update(&mut self, downloaded: u64, total: u64) -> Option<u8> {
        if total == 0 {
            return None;
        }
        let percent = (u128::from(downloaded) * 100 / u128::from(total)).min(100) as u8;
        if self.last == Some(percent) {
            return None;
        }
        self.last = Some(percent);
        Some(percent)
    }
}

/// How long a download may take to reach its server. Past this the host is
/// not answering (blocked, offline, a dead proxy), and waiting longer only
/// leaves the progress bar at 0%.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a download may go without a single byte arriving, before the
/// response headers and between chunks. Not a limit on the whole download: a
/// multi-GB model on a slow link may take an hour as long as it keeps moving.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The whole of a small remote call -- the update feed, the metadata, the dictionary: connecting,
/// the answer and its body. These are a few kilobytes, so unlike a download a host that takes
/// longer is stuck, and must not hold the start-up "checking for updates" forever (review W-8).
pub const REMOTE_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// A response body that went past its cap.
#[derive(Debug, PartialEq, Eq)]
pub struct BodyTooLarge {
    pub limit: usize,
}

impl std::fmt::Display for BodyTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const KIB: usize = 1024;
        const MIB: usize = 1024 * 1024;
        let limit = self.limit;
        if limit >= MIB && limit % MIB == 0 {
            write!(
                f,
                "The response is larger than the {} MiB the app accepts",
                limit / MIB
            )
        } else if limit >= KIB && limit % KIB == 0 {
            write!(
                f,
                "The response is larger than the {} KiB the app accepts",
                limit / KIB
            )
        } else {
            write!(
                f,
                "The response is larger than the {limit} bytes the app accepts"
            )
        }
    }
}

/// Counts the bytes of a response body as its chunks arrive and refuses the one that takes it
/// past `limit`, so a host that never stops sending cannot fill the memory.
#[derive(Debug)]
pub struct BodyCap {
    limit: usize,
    seen: usize,
}

impl BodyCap {
    pub fn new(limit: usize) -> Self {
        Self { limit, seen: 0 }
    }

    /// Adds a chunk of `len` bytes; an error when the body would be larger than the limit.
    pub fn add(&mut self, len: usize) -> Result<(), BodyTooLarge> {
        match self.seen.checked_add(len) {
            Some(total) if total <= self.limit => {
                self.seen = total;
                Ok(())
            }
            _ => Err(BodyTooLarge { limit: self.limit }),
        }
    }

    /// Bytes accepted so far.
    pub fn seen(&self) -> usize {
        self.seen
    }
}

/// Tells a stuck download from a slow one: it is stuck when no data has
/// arrived for `limit`. The clock is passed in, so this is tested without
/// waiting.
#[derive(Debug, Clone)]
pub struct StallWatch {
    limit: Duration,
    last_data: Instant,
}

impl StallWatch {
    pub fn new(limit: Duration, now: Instant) -> Self {
        Self {
            limit,
            last_data: now,
        }
    }

    pub fn limit(&self) -> Duration {
        self.limit
    }

    /// A chunk of `bytes` arrived at `now`. An empty one is not progress: a
    /// server that only trickles empty chunks is still stuck.
    pub fn data_arrived(&mut self, bytes: usize, now: Instant) {
        if bytes > 0 {
            self.last_data = now;
        }
    }

    /// How much longer the next chunk may take; zero once stalled.
    pub fn remaining(&self, now: Instant) -> Duration {
        self.limit
            .saturating_sub(now.saturating_duration_since(self.last_data))
    }

    pub fn is_stalled(&self, now: Instant) -> bool {
        self.remaining(now).is_zero()
    }
}

/// The reason shown to the user (and logged) when a download stalls.
pub fn stall_error(limit: Duration) -> String {
    format!(
        "No data received for {} s; the connection looks stuck.",
        limit.as_secs()
    )
}

/// SHA-256 (lowercase hex) of a file, read in large chunks -- the model is
/// several GB. Run it off the async runtime.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 8 * 1024 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Moves `from` over `to`, retrying while `to` is still locked (on Windows a
/// file stays locked for a moment after the process that mapped it exits).
pub fn replace_file(from: &Path, to: &Path, attempts: u32, pause: Duration) -> io::Result<()> {
    let mut last_err = None;
    for attempt in 0..attempts.max(1) {
        if attempt > 0 {
            std::thread::sleep(pause);
        }
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.expect("at least one attempt"))
}

/// Writes `contents` to `path` so a crash mid-write leaves the old file, not
/// a torn one: the data goes to `<path>.tmp` first and is renamed over `path`.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, contents)?;
    replace_file(&tmp, path, 5, Duration::from_millis(50)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Reads a settings file as text, trying again while it cannot be read (an antivirus scan or
/// another program may hold it for a moment). The last error comes back after `attempts`.
pub fn read_text_retrying(path: &Path, attempts: u32, pause: Duration) -> io::Result<String> {
    let mut last_err = None;
    for attempt in 0..attempts.max(1) {
        if attempt > 0 {
            std::thread::sleep(pause);
        }
        match std::fs::read_to_string(path) {
            Ok(text) => return Ok(text),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.expect("at least one attempt"))
}

/// Copies a file that cannot be used to `<path>.bad` (the next save would replace it) and says
/// where the copy is. The original stays.
pub fn keep_bad_copy(path: &Path) -> io::Result<std::path::PathBuf> {
    keep_copy(path, "bad")
}

/// Copies a file to `<path>.<suffix>` and says where the copy is. The original stays.
pub fn keep_copy(path: &Path, suffix: &str) -> io::Result<std::path::PathBuf> {
    let mut copy = path.as_os_str().to_owned();
    copy.push(".");
    copy.push(suffix);
    let copy = std::path::PathBuf::from(copy);
    std::fs::copy(path, &copy)?;
    Ok(copy)
}

/// `<path>.part`: where something is built before it is moved to `path`.
pub fn part_path(path: &Path) -> std::path::PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    std::path::PathBuf::from(part)
}

/// Moves a finished staging folder to `dest`, so a folder that is there at all is a whole one.
/// Whatever a broken earlier attempt left at `dest` is replaced; nothing is touched when
/// `staged` is missing.
pub fn publish_dir(staged: &Path, dest: &Path) -> io::Result<()> {
    if !staged.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("nothing was staged at {}", staged.display()),
        ));
    }
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::rename(staged, dest)
}

/// The update's swap: the running exe moves to `old`, the downloaded one takes its place.
/// A stale `old` from an earlier update is dropped first. If the second rename fails the
/// first one is undone, so the app is never left without an exe at `current`; the error
/// says what failed and whether the previous version is back.
pub fn install_swap(current: &Path, temp: &Path, old: &Path) -> Result<(), String> {
    if old.exists() {
        let _ = std::fs::remove_file(old);
    }
    std::fs::rename(current, old).map_err(|e| format!("Failed to backup current exe: {}", e))?;
    if let Err(e) = std::fs::rename(temp, current) {
        return Err(match std::fs::rename(old, current) {
            Ok(()) => format!("Failed to install new exe: {}; the previous version was restored", e),
            Err(restore) => format!(
                "Failed to install new exe: {}; restoring the previous version also failed: {} (it is at {})",
                e,
                restore,
                old.display()
            ),
        });
    }
    Ok(())
}

/// Is `remote` a newer version than `current`? Versions that both parse as
/// semver ("0.4.0", "v0.5.1") are compared; anything else falls back to
/// "different means newer", as before.
pub fn is_newer_version(remote: &str, current: &str) -> bool {
    let parse = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v')).ok();
    match (parse(remote), parse(current)) {
        (Some(remote), Some(current)) => remote > current,
        _ => remote != current,
    }
}

/// A download of which only one may run at a time, and that can be cancelled: the app update. A
/// retry started right after a cancel cannot share the `.part` file with the one winding down.
#[derive(Debug, Default)]
pub struct SingleRun {
    running: AtomicBool,
    cancel: AtomicBool,
}

/// Held for as long as the run lasts; dropping it lets the next one start.
#[derive(Debug)]
pub struct RunGuard<'a>(&'a SingleRun);

impl SingleRun {
    /// Starts a run, unless one is running. A new run starts not cancelled.
    pub fn try_start(&self) -> Option<RunGuard<'_>> {
        if self.running.swap(true, Ordering::SeqCst) {
            return None;
        }
        self.cancel.store(false, Ordering::SeqCst);
        Some(RunGuard(self))
    }

    /// Asks the running download to stop; it reads [`SingleRun::cancel_flag`].
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// The flag the running download looks at.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn only_https_urls_are_downloaded() {
        assert!(check_download_url("https://github.com/x/y.zip").is_ok());
        assert!(check_download_url("HTTPS://example.com/a").is_ok());
        assert!(check_download_url("http://example.com/a").is_err());
        assert!(check_download_url("file:///C:/evil.exe").is_err());
        assert!(check_download_url("").is_err());
    }

    #[test]
    fn only_https_pages_are_opened_in_the_browser() {
        // The ui only ever asks for github.com; what else may reach `open_browser` is a script
        // that should not be there (review W-9), so anything but a plain https page is refused.
        assert!(check_open_url("https://github.com/enjay27/resonance-stream").is_ok());
        assert!(check_open_url("HTTPS://github.com/x?y=1#z").is_ok());
        for refused in [
            "http://github.com",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "data:text/html,<script>1</script>",
            "mailto:a@b.c",
            "tel:123",
            "ms-msdt:/id",
            "\\\\server\\share\\x.exe",
            "C:\\Windows\\System32\\calc.exe",
            "",
            "https://",
            "https:///no-host",
            "https://?q=1",
            "https://#frag",
            " https://github.com",
            "https://git hub.com",
            "https://github.com/a\nb",
            "https://github.com/a\0b",
            "https://\\evil.com",
        ] {
            assert!(
                check_open_url(refused).is_err(),
                "{refused:?} must be refused"
            );
        }
    }

    #[test]
    fn a_test_run_may_download_from_this_machine_over_http() {
        let local = "http://127.0.0.1:8099/update.exe";
        assert!(check_download_url(local).is_err());
        assert!(check_download_url_allowing(local, false).is_err());
        assert!(check_download_url_allowing(local, true).is_ok());
        // https works either way; no other plain http does.
        assert!(check_download_url_allowing("https://example.com/a", false).is_ok());
        assert!(check_download_url_allowing("https://example.com/a", true).is_ok());
        for url in [
            "http://example.com/a",
            "http://127.0.0.1.evil.example/a",
            "http://127.0.0.1@evil.example/a",
            "file:///C:/evil.exe",
        ] {
            assert!(check_download_url_allowing(url, true).is_err(), "{url}");
        }
    }

    #[test]
    fn a_matching_download_passes() {
        let mut check = DownloadCheck::new(Some(5), Some(HELLO_SHA256));
        check.update(b"hel");
        check.update(b"lo");
        assert_eq!(check.received(), 5);
        assert!(check.finish().is_ok());
    }

    #[test]
    fn hash_comparison_ignores_case_and_whitespace() {
        let upper = format!(" {} ", HELLO_SHA256.to_uppercase());
        let mut check = DownloadCheck::new(None, Some(&upper));
        check.update(b"hello");
        assert!(check.finish().is_ok());
    }

    #[test]
    fn a_truncated_download_fails() {
        let mut check = DownloadCheck::new(Some(10), None);
        check.update(b"hello");
        assert!(check.finish().unwrap_err().contains("5 of 10"));
    }

    #[test]
    fn a_tampered_download_fails() {
        let mut check = DownloadCheck::new(None, Some(HELLO_SHA256));
        check.update(b"hellO");
        assert!(check.finish().unwrap_err().contains("SHA-256"));
    }

    #[test]
    fn without_expectations_anything_passes() {
        let mut check = DownloadCheck::new(None, None);
        check.update(b"whatever");
        assert!(check.finish().is_ok());
    }

    #[test]
    fn progress_reports_only_changes() {
        let mut p = ProgressThrottle::default();
        assert_eq!(p.update(0, 1000), Some(0));
        assert_eq!(p.update(5, 1000), None); // still 0 %
        assert_eq!(p.update(10, 1000), Some(1));
        assert_eq!(p.update(1000, 1000), Some(100));
        assert_eq!(p.update(1000, 1000), None);
        assert_eq!(p.update(10, 0), None); // unknown total
    }

    #[test]
    fn progress_never_exceeds_100() {
        let mut p = ProgressThrottle::default();
        assert_eq!(p.update(2000, 1000), Some(100));
        assert_eq!(p.update(u64::MAX, u64::MAX), None); // still 100, no overflow
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rs-download-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sha256_file_hashes_the_contents() {
        let dir = temp_dir("sha");
        let path = dir.join("hello.bin");
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(sha256_file(&path).unwrap(), HELLO_SHA256);
        assert!(sha256_file(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let path = dir.join("config.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(!dir.join("config.json.tmp").exists());
        assert!(write_atomic(&dir.join("no/such/dir/x"), b"x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_file_overwrites_the_target() {
        let dir = temp_dir("replace");
        let (new, old) = (dir.join("model.gguf.new"), dir.join("model.gguf"));
        std::fs::write(&new, b"new").unwrap();
        std::fs::write(&old, b"old").unwrap();
        replace_file(&new, &old, 3, Duration::ZERO).unwrap();
        assert_eq!(std::fs::read(&old).unwrap(), b"new");
        assert!(!new.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_file_gives_up_after_its_attempts() {
        let dir = temp_dir("giveup");
        let err = replace_file(&dir.join("missing"), &dir.join("x"), 2, Duration::ZERO);
        assert!(err.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- reading a settings file: a lock is waited out, a bad file is kept (W-7) ---

    #[test]
    fn a_file_that_appears_while_retrying_is_read() {
        // An antivirus scan holds the file for a moment: the read is tried again.
        let dir = temp_dir("read-late");
        let path = dir.join("config.json");
        let late = path.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            std::fs::write(late, b"{}").unwrap();
        });
        let text = read_text_retrying(&path, 100, Duration::from_millis(10)).unwrap();
        writer.join().unwrap();
        assert_eq!(text, "{}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_stays_unreadable_gives_the_last_error_after_its_attempts() {
        let dir = temp_dir("read-never");
        let err = read_text_retrying(&dir.join("missing.json"), 3, Duration::ZERO).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_readable_file_is_read_on_the_first_try() {
        let dir = temp_dir("read-now");
        let path = dir.join("config.json");
        std::fs::write(&path, "hello").unwrap();
        assert_eq!(
            read_text_retrying(&path, 1, Duration::ZERO).unwrap(),
            "hello"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_file_is_kept_beside_the_original() {
        let dir = temp_dir("bad-copy");
        let path = dir.join("config.json");
        std::fs::write(&path, "{ torn").unwrap();
        let kept = keep_bad_copy(&path).unwrap();
        assert_eq!(kept, dir.join("config.json.bad"));
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "{ torn");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ torn",
            "the original is not touched"
        );
        assert!(keep_bad_copy(&dir.join("missing.json")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_can_be_kept_under_any_suffix() {
        let dir = temp_dir("kept-copy");
        let path = dir.join("config.json");
        std::fs::write(&path, "{}").unwrap();
        let kept = keep_copy(&path, "v7").unwrap();
        assert_eq!(kept, dir.join("config.json.v7"));
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "{}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- short remote calls: a limit on the wait and on the body (W-8) ---

    #[test]
    fn a_body_may_be_exactly_as_large_as_its_cap() {
        let mut cap = BodyCap::new(10);
        assert!(cap.add(4).is_ok());
        assert!(cap.add(6).is_ok());
        assert_eq!(cap.seen(), 10);
    }

    #[test]
    fn a_body_one_byte_over_its_cap_is_refused() {
        let mut cap = BodyCap::new(10);
        cap.add(10).unwrap();
        assert_eq!(cap.add(1), Err(BodyTooLarge { limit: 10 }));
    }

    #[test]
    fn a_huge_chunk_cannot_wrap_the_count() {
        let mut cap = BodyCap::new(10);
        cap.add(5).unwrap();
        assert!(cap.add(usize::MAX).is_err());
    }

    #[test]
    fn the_message_names_the_limit() {
        assert_eq!(
            BodyTooLarge { limit: 262_144 }.to_string(),
            "The response is larger than the 256 KiB the app accepts"
        );
        assert_eq!(
            BodyTooLarge {
                limit: 4 * 1024 * 1024
            }
            .to_string(),
            "The response is larger than the 4 MiB the app accepts"
        );
        assert_eq!(
            BodyTooLarge { limit: 1000 }.to_string(),
            "The response is larger than the 1000 bytes the app accepts"
        );
    }

    #[test]
    fn a_short_call_gives_up_before_a_download_would_but_after_connecting() {
        assert!(REMOTE_CALL_TIMEOUT > CONNECT_TIMEOUT);
        assert!(REMOTE_CALL_TIMEOUT <= STALL_TIMEOUT.saturating_mul(2));
    }

    // --- install_swap: the update's two renames --------------------------

    fn swap_paths(
        dir: &std::path::Path,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        (
            dir.join("app.exe"),
            dir.join("update_temp.exe"),
            dir.join("app.exe.old"),
        )
    }

    #[test]
    fn install_swap_puts_the_new_exe_in_place_and_keeps_the_old_one() {
        let dir = temp_dir("swap-ok");
        let (current, temp, old) = swap_paths(&dir);
        std::fs::write(&current, b"v1").unwrap();
        std::fs::write(&temp, b"v2").unwrap();
        install_swap(&current, &temp, &old).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"v2");
        assert_eq!(std::fs::read(&old).unwrap(), b"v1");
        assert!(!temp.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_swap_drops_a_stale_backup_first() {
        let dir = temp_dir("swap-stale");
        let (current, temp, old) = swap_paths(&dir);
        std::fs::write(&current, b"v2").unwrap();
        std::fs::write(&temp, b"v3").unwrap();
        std::fs::write(&old, b"v1").unwrap();
        install_swap(&current, &temp, &old).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"v3");
        assert_eq!(std::fs::read(&old).unwrap(), b"v2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_swap_puts_the_running_exe_back_when_the_second_rename_fails() {
        // W-1: the current exe was already moved to `.old` when installing the new one failed.
        let dir = temp_dir("swap-rollback");
        let (current, temp, old) = swap_paths(&dir);
        std::fs::write(&current, b"v1").unwrap();
        // `update_temp.exe` is gone (an antivirus took it), so the second rename fails.
        let err = install_swap(&current, &temp, &old).unwrap_err();
        assert!(err.contains("Failed to install new exe"), "{err}");
        assert!(err.contains("restored"), "{err}");
        assert_eq!(
            std::fs::read(&current).unwrap(),
            b"v1",
            "the app must still be there"
        );
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_swap_changes_nothing_when_the_first_rename_fails() {
        let dir = temp_dir("swap-first");
        let (current, temp, old) = swap_paths(&dir);
        std::fs::write(&temp, b"v2").unwrap();
        let err = install_swap(&current, &temp, &old).unwrap_err();
        assert!(err.contains("Failed to backup current exe"), "{err}");
        assert_eq!(
            std::fs::read(&temp).unwrap(),
            b"v2",
            "the download is kept for another try"
        );
        assert!(!current.exists() && !old.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- staged folders: an install appears whole or not at all (W-3) -----

    #[test]
    fn a_staging_folder_sits_beside_the_real_one() {
        let dest = std::path::Path::new("bin").join("ai-server");
        assert_eq!(
            part_path(&dest),
            std::path::Path::new("bin").join("ai-server.part")
        );
    }

    #[test]
    fn publish_dir_moves_the_finished_folder_into_place() {
        let dir = temp_dir("publish-ok");
        let (staged, dest) = (dir.join("ai-server.part"), dir.join("ai-server"));
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("llama-server.exe"), b"exe").unwrap();
        std::fs::write(staged.join("ggml.dll"), b"dll").unwrap();
        publish_dir(&staged, &dest).unwrap();
        assert_eq!(
            std::fs::read(dest.join("llama-server.exe")).unwrap(),
            b"exe"
        );
        assert_eq!(std::fs::read(dest.join("ggml.dll")).unwrap(), b"dll");
        assert!(!staged.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_dir_replaces_what_an_earlier_broken_install_left() {
        let dir = temp_dir("publish-replace");
        let (staged, dest) = (dir.join("ai-server.part"), dir.join("ai-server"));
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("llama-server.exe"), b"new").unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("server_temp.zip"), b"leftover").unwrap();
        publish_dir(&staged, &dest).unwrap();
        assert_eq!(
            std::fs::read(dest.join("llama-server.exe")).unwrap(),
            b"new"
        );
        assert!(!dest.join("server_temp.zip").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_dir_leaves_the_destination_alone_when_nothing_was_staged() {
        let dir = temp_dir("publish-missing");
        let (staged, dest) = (dir.join("ai-server.part"), dir.join("ai-server"));
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("keep.txt"), b"keep").unwrap();
        assert!(publish_dir(&staged, &dest).is_err());
        assert_eq!(std::fs::read(dest.join("keep.txt")).unwrap(), b"keep");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- stall detection -------------------------------------------------

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_fresh_watch_allows_the_whole_limit() {
        let t0 = Instant::now();
        let watch = StallWatch::new(secs(30), t0);
        assert_eq!(watch.remaining(t0), secs(30));
        assert!(!watch.is_stalled(t0));
    }

    #[test]
    fn the_wait_counts_down_and_ends_at_the_limit() {
        let t0 = Instant::now();
        let watch = StallWatch::new(secs(30), t0);
        assert_eq!(watch.remaining(t0 + secs(10)), secs(20));
        assert!(!watch.is_stalled(t0 + secs(29)));
        assert!(watch.is_stalled(t0 + secs(30)));
        assert_eq!(watch.remaining(t0 + secs(300)), Duration::ZERO);
    }

    #[test]
    fn data_restarts_the_wait() {
        let t0 = Instant::now();
        let mut watch = StallWatch::new(secs(30), t0);
        watch.data_arrived(1024, t0 + secs(25));
        assert_eq!(watch.remaining(t0 + secs(25)), secs(30));
        assert!(!watch.is_stalled(t0 + secs(54)));
        assert!(watch.is_stalled(t0 + secs(55)));
    }

    #[test]
    fn an_empty_chunk_is_not_progress() {
        let t0 = Instant::now();
        let mut watch = StallWatch::new(secs(30), t0);
        watch.data_arrived(0, t0 + secs(25));
        assert!(watch.is_stalled(t0 + secs(30)));
    }

    #[test]
    fn a_slow_but_steady_download_never_stalls() {
        let t0 = Instant::now();
        let mut watch = StallWatch::new(secs(30), t0);
        for i in 1..=100 {
            let now = t0 + secs(29 * i);
            assert!(!watch.is_stalled(now), "chunk {i}");
            watch.data_arrived(1, now);
        }
    }

    #[test]
    fn a_clock_that_steps_back_does_not_panic() {
        let t0 = Instant::now() + secs(100);
        let watch = StallWatch::new(secs(30), t0);
        assert_eq!(watch.remaining(t0 - secs(50)), secs(30));
        assert!(!watch.is_stalled(t0 - secs(50)));
    }

    #[test]
    fn a_watch_reports_its_limit() {
        assert_eq!(StallWatch::new(secs(12), Instant::now()).limit(), secs(12));
    }

    #[test]
    fn the_stall_message_names_the_wait() {
        let message = stall_error(secs(30));
        assert!(message.contains("30 s"), "{message}");
        assert!(message.to_lowercase().contains("no data"), "{message}");
    }

    #[test]
    fn connecting_gives_up_before_a_stalled_transfer_would() {
        assert!(CONNECT_TIMEOUT <= STALL_TIMEOUT);
        assert!(STALL_TIMEOUT >= secs(10), "a slow link must not be cut off");
    }

    #[test]
    fn version_comparison() {
        // Regression (review R10): an older remote version used to count as an update.
        assert!(!is_newer_version("0.3.9", "0.4.0"));
        assert!(!is_newer_version("0.4.0", "0.4.0"));
        assert!(is_newer_version("0.4.1", "0.4.0"));
        assert!(is_newer_version("v1.0.0", "0.4.0"));
        assert!(is_newer_version("0.10.0", "0.9.0")); // numeric, not string, order
                                                      // Not semver on either side: different means newer, as before.
        assert!(is_newer_version("2026-09", "2026-08"));
        assert!(!is_newer_version("2026-09", "2026-09"));
        assert!(is_newer_version("model-b", ""));
    }

    #[test]
    fn a_second_download_is_refused_while_the_first_runs() {
        let download = SingleRun::default();
        let first = download.try_start();
        assert!(first.is_some());
        assert!(download.is_running());
        assert!(download.try_start().is_none());
    }

    #[test]
    fn a_finished_download_lets_the_next_one_start() {
        let download = SingleRun::default();
        drop(download.try_start());
        assert!(!download.is_running());
        assert!(download.try_start().is_some());
    }

    #[test]
    fn a_cancel_reaches_the_running_download() {
        let download = SingleRun::default();
        let _run = download.try_start();
        assert!(!download.cancel_flag().load(Ordering::SeqCst));
        download.cancel();
        assert!(download.cancel_flag().load(Ordering::SeqCst));
    }

    #[test]
    fn a_new_download_does_not_inherit_an_old_cancel() {
        let download = SingleRun::default();
        download.cancel();
        let _run = download.try_start();
        assert!(!download.cancel_flag().load(Ordering::SeqCst));
    }
}
