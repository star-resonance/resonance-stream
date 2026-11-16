//! Where the in-game module (`resonance_ingame.dll`) comes from and lives: the release
//! it is fetched from (own tag, else the newest rc whose `SHA256SUMS` line matches the
//! exe's pin), its folder under the app's data dir, and the Korean lines of the fetch.
//!
//! Pure: the downloads, the signature check and the folder swap are in
//! `src-tauri/src/services/downloader/ingame_module.rs`. `SHA256SUMS` only picks the
//! release; trust is the signature and the pin, checked by the caller.

use crate::ingame_send::DLL_NAME;
use std::path::{Path, PathBuf};

/// The repository whose releases carry the module.
pub const RELEASE_REPO: &str = "star-resonance/resonance-stream";

/// The `download-progress` label of the module download (the composer matches
/// [`MODULE_LABEL_PREFIX`]).
pub const MODULE_LABEL: &str = "게임 내 전송 모듈 내려받는 중...";
/// What every progress label of the module starts with.
pub const MODULE_LABEL_PREFIX: &str = "게임 내 전송 모듈";

/// Chat system line when the module has been downloaded and is ready.
pub const SUCCESS_LINE: &str = "게임 내 전송 모듈을 내려받았습니다. 이제 채팅을 보낼 수 있습니다.";

/// Why a fetch of the module did not end in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// The release could not be reached.
    Network,
    /// No release holds a module with this exe's pin.
    NotPublished,
    /// The signature or the hash was wrong.
    Rejected,
    /// The file could not be written or moved.
    Disk,
    /// The setting was turned off during the download.
    Cancelled,
}

impl FetchError {
    /// The chat system line for this failure; a cancel says nothing.
    pub fn chat_line(&self) -> Option<&'static str> {
        match self {
            FetchError::Network => Some(
                "게임 내 전송 모듈을 내려받지 못했습니다. 인터넷 연결을 확인한 뒤 설정에서 껐다 켜 주세요.",
            ),
            FetchError::NotPublished => Some(
                "이 앱 버전에 맞는 전송 모듈이 아직 올라와 있지 않습니다. 잠시 뒤 설정에서 껐다 켜 주세요.",
            ),
            FetchError::Rejected => Some(
                "내려받은 전송 모듈을 확인하지 못해 사용하지 않습니다. 설정에서 껐다 켜 주세요. 계속되면 앱을 다시 설치해 주세요.",
            ),
            FetchError::Disk => Some(
                "전송 모듈을 저장하지 못했습니다. 저장 공간을 확인한 뒤 설정에서 껐다 켜 주세요.",
            ),
            FetchError::Cancelled => None,
        }
    }
}

/// The composer's progress sentence for `label` at `percent`.
pub fn progress_line(_label: &str, percent: u32) -> String {
    format!("게임 내 전송 모듈을 내려받는 중... {percent}%")
}

/// The release tag of `version` (`0.6.5` -> `v0.6.5`).
pub fn own_tag(version: &str) -> String {
    format!("v{version}")
}

/// The module's asset name. Every asset of a release, rc included, carries the final
/// version in its name.
pub fn dll_asset(version: &str) -> String {
    format!("Resonance-Stream-v{version}-ingame.dll")
}

/// The module's signature asset.
pub fn sig_asset(version: &str) -> String {
    format!("{}.sig", dll_asset(version))
}

/// The download url of `asset` in release `tag` of `repo`.
pub fn asset_url(repo: &str, tag: &str, asset: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{asset}")
}

/// The release list of `repo` (newest first, 100 at most).
pub fn releases_api_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/releases?per_page=100")
}

/// The tags of the published prereleases `v<version>-rc.<N>` in the release list `json`,
/// highest N first. `Err` when the text is not a list of releases.
pub fn rc_tags(version: &str, json: &str) -> Result<Vec<String>, String> {
    let list: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = list.as_array().ok_or("the release list is not an array")?;
    let prefix = format!("v{version}-rc.");
    let mut found: Vec<(u64, String)> = Vec::new();
    for release in list {
        let flag = |name: &str| release.get(name).and_then(|v| v.as_bool()).unwrap_or(false);
        if flag("draft") || !flag("prerelease") {
            continue;
        }
        let Some(tag) = release.get("tag_name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(n) = tag.strip_prefix(&prefix) else {
            continue;
        };
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Ok(n) = n.parse::<u64>() {
            found.push((n, tag.to_string()));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(found.into_iter().map(|(_, tag)| tag).collect())
}

/// The SHA-256 (lower-case hex) `sums` (`sha256sum` output) lists for `name`.
pub fn sums_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        if file != name || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(hash.to_ascii_lowercase())
    })
}

/// The release to download the module of `version` from: its own tag when that
/// release's `SHA256SUMS` lists the module with `pin`, else the newest rc that does.
///
/// `fetch_sums(tag)`: the `SHA256SUMS` text of a release, `Ok(None)` when it has none
/// (404). `list_releases()`: the release list JSON, asked at most once. An error of
/// either is [`FetchError::Network`]; no match is [`FetchError::NotPublished`].
pub fn resolve_tag(
    pin: &str,
    version: &str,
    mut fetch_sums: impl FnMut(&str) -> Result<Option<String>, String>,
    list_releases: impl FnOnce() -> Result<String, String>,
) -> Result<String, FetchError> {
    let name = dll_asset(version);
    let mut holds_pin = |tag: &str| -> Result<bool, FetchError> {
        let sums = fetch_sums(tag).map_err(|_| FetchError::Network)?;
        Ok(sums
            .and_then(|text| sums_hash(&text, &name))
            .is_some_and(|hash| hash.eq_ignore_ascii_case(pin)))
    };
    let own = own_tag(version);
    if holds_pin(&own)? {
        return Ok(own);
    }
    let list = list_releases().map_err(|_| FetchError::Network)?;
    for tag in rc_tags(version, &list).map_err(|_| FetchError::Network)? {
        if holds_pin(&tag)? {
            return Ok(tag);
        }
    }
    Err(FetchError::NotPublished)
}

/// `<data>\bin\ingame\<version>`: the folder of the module of one version.
pub fn module_dir(data: &Path, version: &str) -> PathBuf {
    data.join("bin").join("ingame").join(version)
}

/// The module's file in [`module_dir`].
pub fn module_file(data: &Path, version: &str) -> PathBuf {
    module_dir(data, version).join(DLL_NAME)
}

/// The file the app loads: the debug override when there is one, else the downloaded
/// module of `version`.
pub fn dll_to_load(override_path: Option<&str>, data: &Path, version: &str) -> PathBuf {
    match override_path {
        Some(path) => PathBuf::from(path),
        None => module_file(data, version),
    }
}

/// Of the entry names under `<data>\bin\ingame`, the ones to remove: a plain
/// `X.Y.Z` folder that is not `own`, and any `X.Y.Z.part` staging folder. Anything else stays.
pub fn stale_entries(names: &[&str], own: &str) -> Vec<String> {
    let plain = |s: &str| {
        let mut parts = s.split('.');
        let mut count = 0;
        for part in parts.by_ref() {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            count += 1;
        }
        count == 3
    };
    names
        .iter()
        .filter(|name| match name.strip_suffix(".part") {
            Some(base) => plain(base),
            None => plain(name) && **name != own,
        })
        .map(|name| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::check_download_url;
    use crate::ingame_send::SendError;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    const PIN: &str = "9fb6788819391714e7d08eab1a0c6b1f5c19b0651d9f80a5afa39d9109b410b2";
    const OTHER: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const DLL: &str = "Resonance-Stream-v0.6.5-ingame.dll";
    const SUMS: &str = "\
1111111111111111111111111111111111111111111111111111111111111111  Resonance-Stream-v0.6.5.exe
2222222222222222222222222222222222222222222222222222222222222222  Resonance-Stream-v0.6.5.exe.sig
9fb6788819391714e7d08eab1a0c6b1f5c19b0651d9f80a5afa39d9109b410b2  Resonance-Stream-v0.6.5-ingame.dll
3333333333333333333333333333333333333333333333333333333333333333  Resonance-Stream-v0.6.5-ingame.dll.sig
";

    fn sums_with(hash: &str) -> String {
        format!("{hash}  {DLL}\n")
    }

    #[test]
    fn names_and_urls_of_the_rc4_release() {
        assert_eq!(own_tag("0.6.5"), "v0.6.5");
        assert_eq!(dll_asset("0.6.5"), DLL);
        assert_eq!(sig_asset("0.6.5"), "Resonance-Stream-v0.6.5-ingame.dll.sig");
        assert_eq!(
            asset_url(RELEASE_REPO, "v0.6.5-rc.4", &dll_asset("0.6.5")),
            "https://github.com/star-resonance/resonance-stream/releases/download/v0.6.5-rc.4/Resonance-Stream-v0.6.5-ingame.dll"
        );
        assert_eq!(
            releases_api_url(RELEASE_REPO),
            "https://api.github.com/repos/star-resonance/resonance-stream/releases?per_page=100"
        );
        for url in [
            asset_url(RELEASE_REPO, &own_tag("0.6.5"), &dll_asset("0.6.5")),
            asset_url(RELEASE_REPO, &own_tag("0.6.5"), &sig_asset("0.6.5")),
            asset_url(RELEASE_REPO, &own_tag("0.6.5"), "SHA256SUMS"),
            releases_api_url(RELEASE_REPO),
        ] {
            assert_eq!(check_download_url(&url), Ok(()), "{url}");
        }
    }

    #[test]
    fn rc_tags_are_the_published_prereleases_of_this_version_newest_first() {
        let json = r#"[
          {"tag_name":"v0.6.5-rc.2","prerelease":true,"draft":false},
          {"tag_name":"v0.6.5-rc.10","prerelease":true,"draft":false},
          {"tag_name":"v0.6.5-rc.4","prerelease":true,"draft":false},
          {"tag_name":"v0.6.4","prerelease":false,"draft":false},
          {"tag_name":"v0.6.5-rc.5","prerelease":true,"draft":true},
          {"tag_name":"v0.6.50-rc.1","prerelease":true,"draft":false},
          {"tag_name":"v0.6.5-rc.x","prerelease":true,"draft":false},
          {"tag_name":"v0.6.5","prerelease":false,"draft":false}
        ]"#;
        assert_eq!(
            rc_tags("0.6.5", json).unwrap(),
            ["v0.6.5-rc.10", "v0.6.5-rc.4", "v0.6.5-rc.2"]
        );
        assert_eq!(rc_tags("0.6.5", "[]").unwrap(), Vec::<String>::new());
        assert!(rc_tags("0.6.5", "not json").is_err());
    }

    #[test]
    fn sums_hash_reads_one_line_by_name() {
        assert_eq!(sums_hash(SUMS, DLL).as_deref(), Some(PIN));
        assert_eq!(
            sums_hash(SUMS, "Resonance-Stream-v0.6.5-ingame.dll.sig").as_deref(),
            Some("3333333333333333333333333333333333333333333333333333333333333333")
        );
        assert_eq!(sums_hash(SUMS, "other.dll"), None);
        assert_eq!(sums_hash("", DLL), None);
        assert_eq!(sums_hash(&sums_with(&PIN[..63]), DLL), None);
        assert_eq!(
            sums_hash(&format!("{PIN}  *{DLL}\n"), DLL).as_deref(),
            Some(PIN)
        );
        assert_eq!(
            sums_hash(&sums_with(&PIN.to_uppercase()), DLL).as_deref(),
            Some(PIN)
        );
    }

    /// Fakes: `sums` answers per tag; `list` is the release list text.
    fn resolve(
        sums: &[(&str, Result<Option<String>, String>)],
        list: Result<String, String>,
        asked: &RefCell<Vec<String>>,
        list_calls: &RefCell<u32>,
    ) -> Result<String, FetchError> {
        resolve_tag(
            PIN,
            "0.6.5",
            |tag| {
                asked.borrow_mut().push(tag.to_string());
                sums.iter()
                    .find(|(t, _)| *t == tag)
                    .map(|(_, r)| r.clone())
                    .unwrap_or(Ok(None))
            },
            || {
                *list_calls.borrow_mut() += 1;
                list.clone()
            },
        )
    }

    const LIST: &str = r#"[
      {"tag_name":"v0.6.5-rc.4","prerelease":true,"draft":false},
      {"tag_name":"v0.6.5-rc.5","prerelease":true,"draft":false}
    ]"#;

    #[test]
    fn the_own_tag_is_used_when_it_holds_the_pin() {
        let asked = RefCell::new(vec![]);
        let calls = RefCell::new(0);
        let sums = [("v0.6.5", Ok(Some(sums_with(PIN))))];
        assert_eq!(
            resolve(&sums, Ok(LIST.into()), &asked, &calls),
            Ok("v0.6.5".to_string())
        );
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn a_missing_own_tag_falls_back_to_the_rc_holding_the_pin() {
        let asked = RefCell::new(vec![]);
        let calls = RefCell::new(0);
        let sums = [
            ("v0.6.5-rc.5", Ok(Some(sums_with(OTHER)))),
            ("v0.6.5-rc.4", Ok(Some(sums_with(PIN)))),
        ];
        assert_eq!(
            resolve(&sums, Ok(LIST.into()), &asked, &calls),
            Ok("v0.6.5-rc.4".to_string())
        );
        assert_eq!(
            *asked.borrow(),
            ["v0.6.5", "v0.6.5-rc.5", "v0.6.5-rc.4"].map(String::from)
        );
    }

    #[test]
    fn an_own_tag_with_another_hash_falls_through_to_the_list() {
        let asked = RefCell::new(vec![]);
        let calls = RefCell::new(0);
        let sums = [
            ("v0.6.5", Ok(Some(sums_with(OTHER)))),
            ("v0.6.5-rc.4", Ok(Some(sums_with(PIN)))),
        ];
        assert_eq!(
            resolve(&sums, Ok(LIST.into()), &asked, &calls),
            Ok("v0.6.5-rc.4".to_string())
        );
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn nothing_matching_is_not_published_and_errors_are_network() {
        let asked = RefCell::new(vec![]);
        let calls = RefCell::new(0);
        assert_eq!(
            resolve(&[], Ok(LIST.into()), &asked, &calls),
            Err(FetchError::NotPublished)
        );
        let sums = [("v0.6.5", Err("boom".to_string()))];
        assert_eq!(
            resolve(&sums, Ok(LIST.into()), &asked, &calls),
            Err(FetchError::Network)
        );
        assert_eq!(
            resolve(&[], Err("boom".into()), &asked, &calls),
            Err(FetchError::Network)
        );
        let sums = [("v0.6.5-rc.5", Err("boom".to_string()))];
        assert_eq!(
            resolve(&sums, Ok(LIST.into()), &asked, &calls),
            Err(FetchError::Network)
        );
    }

    #[test]
    fn paths_follow_the_version() {
        let data = Path::new("D");
        assert_eq!(
            module_dir(data, "0.6.5"),
            PathBuf::from("D").join("bin").join("ingame").join("0.6.5")
        );
        assert!(module_file(data, "0.6.5").ends_with("resonance_ingame.dll"));
        assert_eq!(
            dll_to_load(Some("X.dll"), data, "0.6.5"),
            PathBuf::from("X.dll")
        );
        assert_eq!(dll_to_load(None, data, "0.6.5"), module_file(data, "0.6.5"));
    }

    #[test]
    fn stale_entries_are_other_plain_versions_and_their_part_folders() {
        let names = [
            "0.6.4",
            "0.6.5",
            "0.6.6",
            "0.6.5.part",
            "0.6.4.part",
            "notes.txt",
            "0.6.5-rc.1",
        ];
        assert_eq!(
            stale_entries(&names, "0.6.5"),
            ["0.6.4", "0.6.6", "0.6.5.part", "0.6.4.part"]
        );
    }

    #[test]
    fn fetch_sentences_are_plain_and_distinct() {
        let errors = [
            FetchError::Network,
            FetchError::NotPublished,
            FetchError::Rejected,
            FetchError::Disk,
        ];
        let mut seen: Vec<String> = vec![];
        for e in errors {
            let s = e.chat_line().expect("a sentence").to_string();
            assert!(!s.contains('\n'));
            assert!(!s.bytes().any(|b| b.is_ascii_digit()), "{s}");
            for bad in ["\\", "http", "sha", "SHA"] {
                assert!(!s.contains(bad), "{s}");
            }
            assert!(!seen.contains(&s));
            seen.push(s);
        }
        assert_eq!(FetchError::Cancelled.chat_line(), None);
        for s in &seen {
            for send in [
                SendError::NoPin,
                SendError::DllMissing,
                SendError::PinMismatch,
                SendError::DllDownloading,
                SendError::LoadFailed,
            ] {
                assert_ne!(*s, send.to_string());
            }
        }
        assert_eq!(
            SUCCESS_LINE,
            "게임 내 전송 모듈을 내려받았습니다. 이제 채팅을 보낼 수 있습니다."
        );
    }

    #[test]
    fn the_progress_line_shows_the_percent() {
        assert!(progress_line(MODULE_LABEL, 40).contains("40%"));
        assert_eq!(MODULE_LABEL, "게임 내 전송 모듈 내려받는 중...");
    }
}
