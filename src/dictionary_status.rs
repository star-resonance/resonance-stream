//! The line in the settings that says which dictionary is in use: the version last synced, the
//! revision of the signed metadata it came from, and whether the file still is that one. Pure and
//! host-tested; the backend's `get_dictionary_status` supplies the numbers.

use crate::ui_types::{DictionaryState, DictionaryStatus};

/// What the settings show next to the sync button, e.g. `v1.0.8 · 서명 리비전 3 · 게시본과 같음`.
pub fn status_line(status: &DictionaryStatus) -> String {
    let version = status.version.trim();
    // `0.0.0` is what the metadata file starts with: no sync has happened.
    let never_synced = version.is_empty() || version == "0.0.0";
    let mut parts: Vec<String> = Vec::new();
    if never_synced {
        parts.push("아직 동기화한 적 없음".into());
        if status.state == DictionaryState::Modified {
            parts.push("직접 수정됨".into());
        }
    } else {
        parts.push(format!("v{version}"));
        if status.revision > 0 {
            parts.push(format!("서명 리비전 {}", status.revision));
        }
        parts.push(
            match status.state {
                DictionaryState::Same => "게시본과 같음",
                DictionaryState::Modified => "직접 수정됨",
                DictionaryState::Unknown => "확인 불가",
            }
            .into(),
        );
    }
    parts.join(" · ")
}

/// How a dictionary sync ended, as the settings show it next to the button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    pub ok: bool,
    pub text: String,
}

/// The outcome of a sync: `Ok` when the backend installed the dictionary, `Err` with what it (or the
/// check before it) said otherwise. The reason is shown as it came; only an empty one is left out.
pub fn sync_outcome(result: Result<(), String>) -> SyncOutcome {
    match result {
        Ok(()) => SyncOutcome {
            ok: true,
            text: "사전을 최신 상태로 동기화했습니다.".into(),
        },
        Err(reason) => {
            let reason = reason.trim();
            SyncOutcome {
                ok: false,
                text: if reason.is_empty() {
                    "동기화 실패".into()
                } else {
                    format!("동기화 실패: {reason}")
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(version: &str, revision: u64, state: DictionaryState) -> DictionaryStatus {
        DictionaryStatus {
            version: version.into(),
            revision,
            state,
        }
    }

    #[test]
    fn a_synced_dictionary_names_its_version_revision_and_state() {
        assert_eq!(
            status_line(&status("1.0.8", 3, DictionaryState::Same)),
            "v1.0.8 · 서명 리비전 3 · 게시본과 같음"
        );
        assert_eq!(
            status_line(&status("1.0.8", 3, DictionaryState::Modified)),
            "v1.0.8 · 서명 리비전 3 · 직접 수정됨"
        );
    }

    #[test]
    fn an_unknown_revision_is_left_out_not_shown_as_zero() {
        assert_eq!(
            status_line(&status("1.0.6", 0, DictionaryState::Unknown)),
            "v1.0.6 · 확인 불가"
        );
    }

    #[test]
    fn a_dictionary_that_was_never_synced_says_so() {
        for version in ["", "0.0.0", "  "] {
            assert_eq!(
                status_line(&status(version, 0, DictionaryState::Unknown)),
                "아직 동기화한 적 없음"
            );
        }
        // Edited before any sync: still never synced, and it does not claim a version.
        assert_eq!(
            status_line(&status("0.0.0", 0, DictionaryState::Modified)),
            "아직 동기화한 적 없음 · 직접 수정됨"
        );
    }

    #[test]
    fn a_sync_that_worked_says_so() {
        let outcome = sync_outcome(Ok(()));
        assert!(outcome.ok);
        assert!(outcome.text.contains("동기화"));
    }

    #[test]
    fn a_sync_that_failed_shows_the_backends_reason() {
        let outcome = sync_outcome(Err(
            " The dictionary was refused: its hash is not the signed one. ".into(),
        ));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.text,
            "동기화 실패: The dictionary was refused: its hash is not the signed one."
        );
    }

    #[test]
    fn a_failure_with_no_reason_is_still_a_failure() {
        for reason in ["", "   "] {
            let outcome = sync_outcome(Err(reason.into()));
            assert_eq!((outcome.ok, outcome.text.as_str()), (false, "동기화 실패"));
        }
    }
}
