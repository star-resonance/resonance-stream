//! What the first-run wizard downloads, read from the update check, and what it tells the person when
//! it cannot. Pure: the wizard (`app/setup_flow.rs`) passes the check's answer in and shows the text.

use crate::ui_types::UpdateCheckResult;

/// Whether the wizard can start the model download, or why not. The backend takes the model's address
/// and hash from the verified metadata itself; this only says the check found them. A refused publication
/// (`metadata_error`: the model's address and hash could not be trusted) is said as that, not as the
/// empty hash it leaves behind.
pub fn plan_downloads(check: &UpdateCheckResult) -> Result<(), String> {
    if let Some(reason) = &check.metadata_error {
        return Err(format!(
            "모델 정보의 서명을 확인하지 못해 설치를 시작하지 않았습니다. ({reason})"
        ));
    }
    let model = &check.remote_data.model;
    if model.download_url.trim().is_empty() || model.sha256.trim().is_empty() {
        return Err(
            "모델 정보가 비어 있어 설치를 시작하지 않았습니다. 잠시 후 다시 시도해 주세요."
                .to_string(),
        );
    }
    Ok(())
}

/// The text for a check that could not be made at all (`reason` is what the backend said).
pub fn check_failed_message(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "모델 정보를 가져오지 못했습니다. 인터넷 연결을 확인하고 다시 시도해 주세요.".to_string()
    } else {
        format!("모델 정보를 가져오지 못했습니다. 인터넷 연결을 확인하고 다시 시도해 주세요. ({reason})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(json: &str) -> UpdateCheckResult {
        serde_json::from_str(json).expect("a check result")
    }

    const GOOD: &str = r#"{"app_update_available":false,"model_update_available":true,"dict_update_available":true,
        "remote_data":{"revision":4,
            "model":{"latest_version":"1.1.0","download_url":"https://example.com/m.gguf","release_notes":"","sha256":"ab12"},
            "dictionary":{"version":"1.0.6","updated_at":"2026-03-08","sha256":"cd34"}}}"#;

    #[test]
    fn a_good_check_lets_the_wizard_start_the_download() {
        // Only a go: the backend takes the model's address and hash from the verified metadata itself (N-5).
        assert_eq!(plan_downloads(&checked(GOOD)), Ok(()));
    }

    #[test]
    fn a_refused_publication_stops_the_wizard_and_says_why() {
        let refused = checked(
            r#"{"app_update_available":false,"model_update_available":false,"dict_update_available":false,
                "remote_data":{"model":{"latest_version":"","download_url":"","release_notes":"","sha256":""},
                               "dictionary":{"version":"","updated_at":""}},
                "metadata_error":"Model and dictionary updates were refused: no signature is published for the metadata."}"#,
        );
        let message = plan_downloads(&refused).unwrap_err();
        // Not "No SHA-256 published": the person is told the model's information could not be trusted, and the reason.
        assert!(message.contains("서명"), "{message}");
        assert!(message.contains("no signature is published"), "{message}");
        assert!(!message.contains("SHA-256"), "{message}");
    }

    #[test]
    fn a_check_with_no_model_address_or_no_hash_stops_the_wizard() {
        for (field, value) in [("download_url", ""), ("sha256", "")] {
            let json = GOOD.replace(
                &format!(
                    "\"{field}\":\"{}\"",
                    if field == "sha256" {
                        "ab12"
                    } else {
                        "https://example.com/m.gguf"
                    }
                ),
                &format!("\"{field}\":\"{value}\""),
            );
            let message = plan_downloads(&checked(&json)).unwrap_err();
            assert!(message.contains("모델"), "{field}: {message}");
        }
    }

    #[test]
    fn a_check_that_could_not_be_made_says_what_went_wrong() {
        let message = check_failed_message("Network error: dns error");
        assert!(message.contains("Network error: dns error"), "{message}");
        assert!(message.contains("인터넷"), "{message}");
        // A reason that is not text (the bridge gave nothing readable) still gives a sentence.
        assert!(check_failed_message("").contains("인터넷"));
    }
}
