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
