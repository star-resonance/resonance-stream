//! The app's update feed: `latest.json`, published with every stable release
//! (`promote.yml`; shape built by `.github/scripts/release-lib.sh`).
//!
//! The feed itself is not trusted -- it is a file on a release page. What makes
//! an update safe is the signature it carries, checked by
//! [`crate::update_signature`] against keys built into the app. This module
//! only turns the text into a value and refuses what could not be an update.

use crate::download::check_download_url_allowing;

/// What the feed announces: the newest stable release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFeed {
    /// Without a leading `v`; valid semver.
    pub version: String,
    pub notes: String,
    /// HTTPS URL of the exe.
    pub url: String,
    /// Base64 minisign signature of the exe, as `tauri signer sign` writes it.
    pub signature: String,
}

/// Reads `latest.json`. `pub_date` and any other field are ignored.
pub fn parse_feed(json: &str) -> Result<UpdateFeed, String> {
    parse_feed_allowing(json, false)
}

/// [`parse_feed`]; with `allow_local_http` the exe's URL may be plain `http://`
/// to this machine (a test run's mock server, see `check_download_url_allowing`).
pub fn parse_feed_allowing(json: &str, allow_local_http: bool) -> Result<UpdateFeed, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("the update feed is not JSON: {e}"))?;
    let text = |name: &str| -> Result<&str, String> {
        value
            .get(name)
            .ok_or_else(|| format!("the update feed has no \"{name}\""))?
            .as_str()
            .ok_or_else(|| format!("the update feed's \"{name}\" is not text"))
    };

    let announced = text("version")?;
    let version = announced.trim().trim_start_matches('v');
    if semver::Version::parse(version).is_err() {
        return Err(format!(
            "the update feed's version {announced:?} is not a version number"
        ));
    }
    let url = text("url")?;
    check_download_url_allowing(url, allow_local_http)?;
    let signature = text("signature")?.trim();
    if signature.is_empty() {
        return Err("the update feed has an empty signature".to_string());
    }
    let notes = value.get("notes").and_then(|n| n.as_str()).unwrap_or("");

    Ok(UpdateFeed {
        version: version.to_string(),
        notes: notes.to_string(),
        url: url.to_string(),
        signature: signature.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r###"{
        "version": "0.6.1",
        "notes": "## 변경 사항\n- one",
        "pub_date": "2026-10-02T09:00:00Z",
        "url": "https://github.com/o/r/releases/download/v0.6.1/Resonance-Stream-v0.6.1.exe",
        "signature": "c2lnbmF0dXJl"
    }"###;

    #[test]
    fn a_local_http_exe_url_is_read_only_when_a_test_run_allows_it() {
        let local = FULL.replace(
            "https://github.com/o/r/releases/download/v0.6.1/Resonance-Stream-v0.6.1.exe",
            "http://127.0.0.1:8099/Resonance-Stream-v0.6.1.exe",
        );
        assert!(parse_feed(&local).is_err());
        assert!(parse_feed_allowing(&local, false).is_err());
        let feed = parse_feed_allowing(&local, true).expect("local http allowed");
        assert_eq!(
            feed.url,
            "http://127.0.0.1:8099/Resonance-Stream-v0.6.1.exe"
        );

        let remote = FULL.replace("https://github.com", "http://github.com");
        assert!(parse_feed_allowing(&remote, true).is_err());
        assert_eq!(parse_feed_allowing(FULL, false), parse_feed(FULL));
    }

    #[test]
    fn a_full_feed_is_read() {
        assert_eq!(
            parse_feed(FULL),
            Ok(UpdateFeed {
                version: "0.6.1".into(),
                notes: "## 변경 사항\n- one".into(),
                url: "https://github.com/o/r/releases/download/v0.6.1/Resonance-Stream-v0.6.1.exe"
                    .into(),
                signature: "c2lnbmF0dXJl".into(),
            })
        );
    }

    #[test]
    fn a_leading_v_is_dropped_from_the_version() {
        let json = FULL.replace("\"0.6.1\"", "\"v0.6.1\"");
        assert_eq!(parse_feed(&json).unwrap().version, "0.6.1");
    }

    #[test]
    fn notes_are_optional() {
        let json = r#"{"version":"0.6.1","url":"https://x/y.exe","signature":"c2ln"}"#;
        assert_eq!(parse_feed(json).unwrap().notes, "");
    }

    #[test]
    fn the_signature_loses_stray_whitespace() {
        let json = FULL.replace("c2lnbmF0dXJl", "c2lnbmF0dXJl\\n");
        assert_eq!(parse_feed(&json).unwrap().signature, "c2lnbmF0dXJl");
    }

    #[test]
    fn a_missing_field_is_named() {
        for field in ["version", "url", "signature"] {
            let mut value: serde_json::Value = serde_json::from_str(FULL).unwrap();
            value.as_object_mut().unwrap().remove(field);
            let err = parse_feed(&value.to_string()).unwrap_err();
            assert!(err.contains(field), "{field}: {err}");
        }
    }

    #[test]
    fn an_exe_that_is_not_https_is_refused() {
        let json = FULL.replace("https://github.com", "http://github.com");
        assert!(parse_feed(&json).unwrap_err().contains("HTTPS"));
    }

    #[test]
    fn an_empty_signature_is_refused() {
        let json = FULL.replace("c2lnbmF0dXJl", "  ");
        assert!(parse_feed(&json).unwrap_err().contains("signature"));
    }

    #[test]
    fn a_version_that_is_not_semver_is_refused() {
        for bad in ["latest", "", "1.2", "1.2.3.4"] {
            let json = FULL.replace("0.6.1\",", &format!("{bad}\","));
            assert!(
                parse_feed(&json).unwrap_err().contains("version"),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_field_of_the_wrong_type_is_refused() {
        let json = FULL.replace("\"0.6.1\"", "6");
        assert!(parse_feed(&json).unwrap_err().contains("version"));
    }

    #[test]
    fn text_that_is_not_json_is_refused() {
        assert!(parse_feed("<html>404</html>").is_err());
        assert!(parse_feed("").is_err());
        assert!(parse_feed("[]").is_err());
    }
}
