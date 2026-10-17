//! The metadata the app reads from this repo's `metadata` branch -- the model's version, URL and hash,
//! and the custom dictionary's version and hash -- is accepted only when it is signed.
//!
//! The file is signed with the same minisign keys as an app update (`update_signature`), so whoever can
//! edit the file or the host it is served from cannot make the app download another model: they would
//! need the private key. The signed comment carries `version:<revision>`, and the file carries the same
//! `revision`: a signature made for one revision does not pass for another. The app also remembers the
//! highest revision it accepted and refuses a lower one, so an old signed file replayed later cannot
//! put an old model or dictionary back. The dictionary has no signature of its own: the signed
//! metadata names its SHA-256, and [`verify_dictionary`] checks the bytes against it.
//!
//! Pure: no clock, no network, no files; the caller passes the bytes and the last accepted revision.

use crate::update_signature::{verify_update, UpdateSignatureError, TRUSTED_UPDATE_KEYS};
use resonance_types::{DictionaryState, GistMetadata};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum MetadataError {
    /// The body is not the JSON the app expects.
    NotJson(String),
    /// The file has no `revision` (a positive whole number): it is not a signed metadata file.
    NoRevision,
    /// No signature was published next to the metadata (its `.sig` is not there).
    NoSignature,
    /// The signature is malformed, from another key, or for another revision.
    Signature(UpdateSignatureError),
    /// A correctly signed file, but older than one the app already accepted.
    Older { found: u64, accepted: u64 },
    /// The metadata does not name the dictionary's hash, so it cannot vouch for a dictionary.
    NoDictionaryHash,
    /// The dictionary is not the one the signed metadata names.
    DictionaryDiffers { expected: String, found: String },
    /// The dictionary is the named one, but the app cannot read it as a dictionary.
    DictionaryUnreadable(String),
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson(why) => write!(f, "the metadata is not readable JSON ({why})"),
            Self::NoRevision => write!(f, "the metadata names no revision, so it cannot be a signed file"),
            Self::NoSignature => write!(f, "no signature is published for the metadata"),
            Self::Signature(why) => write!(f, "the metadata signature was refused: {why}"),
            Self::Older { found, accepted } => write!(
                f,
                "the metadata is revision {found}, older than the revision {accepted} already accepted"
            ),
            Self::NoDictionaryHash => {
                write!(f, "the metadata does not name the dictionary's hash, so the dictionary is not trusted")
            }
            Self::DictionaryDiffers { expected, found } => write!(
                f,
                "the dictionary does not match the signed metadata (expected {expected}, got {found})"
            ),
            Self::DictionaryUnreadable(why) => {
                write!(f, "the dictionary matches its hash but is not a readable dictionary ({why})")
            }
        }
    }
}

/// The metadata in `body`, if `signature_b64` is a signature from one of `trusted_keys` for the
/// revision the file names, and that revision is not older than `accepted_revision` (the highest the
/// app has accepted; 0 before the first).
pub fn verify_metadata(
    body: &[u8],
    signature_b64: &str,
    trusted_keys: &[&str],
    accepted_revision: u64,
) -> Result<GistMetadata, MetadataError> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| MetadataError::NotJson(e.to_string()))?;
    // Read only to know which revision the signature must name; nothing else of the file is used
    // before the signature has been checked.
    let revision = value
        .get("revision")
        .and_then(serde_json::Value::as_u64)
        .filter(|&revision| revision > 0)
        .ok_or(MetadataError::NoRevision)?;
    verify_update(body, signature_b64, trusted_keys, &revision.to_string())
        .map_err(MetadataError::Signature)?;
    if revision < accepted_revision {
        return Err(MetadataError::Older {
            found: revision,
            accepted: accepted_revision,
        });
    }
    serde_json::from_value(value).map_err(|e| MetadataError::NotJson(e.to_string()))
}

/// A whole publication the way the app takes it: the metadata, then the dictionary it names, which the
/// app must also be able to read. `metadata.yml` runs this on the freshly signed files before it
/// publishes them.
pub fn verify_published(
    metadata: &[u8],
    signature_b64: &str,
    dictionary: &[u8],
    trusted_keys: &[&str],
    accepted_revision: u64,
) -> Result<GistMetadata, MetadataError> {
    let verified = verify_metadata(metadata, signature_b64, trusted_keys, accepted_revision)?;
    accept_dictionary(dictionary, &verified)?;
    Ok(verified)
}

/// Where the signature of the metadata at `metadata_url` is read from: next to it, with `.sig`
/// added (what `metadata.yml` publishes).
pub fn signature_url(metadata_url: &str) -> String {
    format!("{metadata_url}.sig")
}

/// [`verify_metadata`] for a signature that may not have been published at all (`None`).
pub fn accept_metadata(
    body: &[u8],
    signature_b64: Option<&str>,
    trusted_keys: &[&str],
    accepted_revision: u64,
) -> Result<GistMetadata, MetadataError> {
    let signature_b64 = signature_b64.ok_or(MetadataError::NoSignature)?;
    verify_metadata(body, signature_b64, trusted_keys, accepted_revision)
}

/// A dictionary the (already verified) `metadata` names, which the app can also read.
pub fn accept_dictionary(dictionary: &[u8], metadata: &GistMetadata) -> Result<(), MetadataError> {
    verify_dictionary(dictionary, metadata)?;
    check_dictionary_reads(dictionary)
}

/// The keys a metadata signature is checked against: the ones built into the app, or -- in a test
/// run that names one (`--metadata-trust-key`) -- that key alone. It replaces the app keys and does
/// not add to them, so a test can also show that a publication signed by the real key is refused.
pub fn trusted_metadata_keys(test_key: Option<&str>) -> Vec<&str> {
    match test_key {
        Some(key) => vec![key],
        None => TRUSTED_UPDATE_KEYS.to_vec(),
    }
}

/// The system-log line for a publication that was refused: what was not updated, why, and that the
/// copy already installed is left alone.
pub fn refusal_line(error: &MetadataError) -> String {
    format!(
        "Model and dictionary updates were refused: {error}. The installed model and dictionary are kept."
    )
}

/// Where the model comes from: what the verified metadata names, never what a caller hands over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSource {
    pub url: String,
    pub version: String,
    /// SHA-256 (hex) the downloaded file must have.
    pub sha256: String,
}

/// The model entry of verified metadata as something to download. A model with no published
/// SHA-256 or no address is refused before anything is downloaded.
pub fn model_source(metadata: &GistMetadata) -> Result<ModelSource, String> {
    let model = &metadata.model;
    let sha256 = model.sha256.trim();
    if sha256.is_empty() {
        return Err("No SHA-256 published for this model; refusing to download it".into());
    }
    let url = model.download_url.trim();
    if url.is_empty() {
        return Err("No download address published for this model; refusing to download it".into());
    }
    Ok(ModelSource {
        url: url.to_string(),
        version: model.latest_version.clone(),
        sha256: sha256.to_string(),
    })
}

/// Can the app read these bytes as its dictionary (text, and the categorised JSON)?
pub fn check_dictionary_reads(dictionary: &[u8]) -> Result<(), MetadataError> {
    let text = std::str::from_utf8(dictionary)
        .map_err(|_| MetadataError::DictionaryUnreadable("not UTF-8 text".to_string()))?;
    crate::text::Dictionary::from_json_str(text)
        .map(|_| ())
        .map_err(MetadataError::DictionaryUnreadable)
}

/// [`verify_metadata`] against the keys built into the app.
pub fn verify_metadata_with_app_keys(
    body: &[u8],
    signature_b64: &str,
    accepted_revision: u64,
) -> Result<GistMetadata, MetadataError> {
    verify_metadata(body, signature_b64, TRUSTED_UPDATE_KEYS, accepted_revision)
}

/// Is `dictionary` the file the (already verified) `metadata` names by its SHA-256?
pub fn verify_dictionary(dictionary: &[u8], metadata: &GistMetadata) -> Result<(), MetadataError> {
    let expected = metadata.dictionary.sha256.trim().to_ascii_lowercase();
    if expected.is_empty() {
        return Err(MetadataError::NoDictionaryHash);
    }
    let found: String = Sha256::digest(dictionary)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if found == expected {
        Ok(())
    } else {
        Err(MetadataError::DictionaryDiffers { expected, found })
    }
}

/// Is the dictionary file on disk the one that was synced? `file` is its SHA-256 (`None`: there is no
/// file), `recorded` the one kept at the last sync. Without both there is nothing to compare, and
/// that is `Unknown` -- never `Same`, so two empty hashes do not agree.
pub fn dictionary_state(file: Option<&str>, recorded: &str) -> DictionaryState {
    let recorded = recorded.trim().to_ascii_lowercase();
    let Some(file) = file.map(|hash| hash.trim().to_ascii_lowercase()) else {
        return DictionaryState::Unknown;
    };
    if file.is_empty() || recorded.is_empty() {
        DictionaryState::Unknown
    } else if file == recorded {
        DictionaryState::Same
    } else {
        DictionaryState::Modified
    }
}

/// The system-log line for a dictionary that was refused although the metadata was accepted. A hash
/// mismatch is often only timing -- a file published a moment ago can still be an old copy on some
/// server -- so it says to try again later.
pub fn dictionary_refusal_line(error: &MetadataError) -> String {
    let retry = match error {
        MetadataError::DictionaryDiffers { .. } => {
            " A file published a moment ago can take a few minutes to reach every server; try again later."
        }
        _ => "",
    };
    format!("The dictionary was refused: {error}. The installed dictionary is kept.{retry}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn published(model: &str) -> GistMetadata {
        serde_json::from_str(&format!(
            r#"{{"revision":3,"model":{model},"dictionary":{{"version":"1.0.8","updated_at":"2026-10-07","sha256":"cd34"}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn the_model_is_downloaded_from_what_the_signed_metadata_names() {
        let verified = published(
            r#"{"latest_version":"1.1.0","download_url":" https://example.com/m.gguf ","release_notes":"","sha256":" AB12 "}"#,
        );
        assert_eq!(
            model_source(&verified),
            Ok(ModelSource {
                url: "https://example.com/m.gguf".into(),
                version: "1.1.0".into(),
                sha256: "AB12".into(),
            })
        );
    }

    #[test]
    fn a_model_with_no_published_hash_or_address_is_refused() {
        let no_hash = published(
            r#"{"latest_version":"1.1.0","download_url":"https://example.com/m.gguf","release_notes":"","sha256":"  "}"#,
        );
        let err = model_source(&no_hash).unwrap_err();
        assert!(err.contains("No SHA-256 published"), "{err}");
        let no_url = published(
            r#"{"latest_version":"1.1.0","download_url":"","release_notes":"","sha256":"ab12"}"#,
        );
        let err = model_source(&no_url).unwrap_err();
        assert!(err.contains("No download address"), "{err}");
    }

    #[test]
    fn the_dictionary_on_disk_is_the_synced_one_only_when_the_hashes_agree() {
        let recorded = "ab12cd";
        assert_eq!(
            dictionary_state(Some("ab12cd"), recorded),
            DictionaryState::Same
        );
        // Case and stray whitespace in either do not make a file "modified".
        assert_eq!(
            dictionary_state(Some("AB12CD\n"), " ab12cd "),
            DictionaryState::Same
        );
        assert_eq!(
            dictionary_state(Some("ab12ce"), recorded),
            DictionaryState::Modified
        );
    }

    #[test]
    fn with_nothing_to_compare_the_dictionary_is_not_claimed_to_be_the_same() {
        // No file, or no hash recorded (never synced; a copy from before the hash was kept): unknown,
        // never "same" -- two empty strings must not agree.
        assert_eq!(dictionary_state(None, "ab12cd"), DictionaryState::Unknown);
        assert_eq!(
            dictionary_state(Some("ab12cd"), ""),
            DictionaryState::Unknown
        );
        assert_eq!(
            dictionary_state(Some("ab12cd"), "  "),
            DictionaryState::Unknown
        );
        assert_eq!(dictionary_state(Some(""), ""), DictionaryState::Unknown);
        assert_eq!(dictionary_state(None, ""), DictionaryState::Unknown);
    }

    // Throwaway keys and signatures made with the `minisign` crate (what `tauri signer` uses); they
    // sign nothing real. BODY is what was signed; SIG_<n>_<key> is its signature for revision <n>.
    const BODY: &[u8] = br#"{"revision":7,"model":{"latest_version":"1.1.0","download_url":"https://example.invalid/model.gguf","release_notes":"test model","sha256":"aa"},"dictionary":{"version":"1.0.6","updated_at":"2026-03-08","sha256":"ec162304d9e0845a552e329f270d0f85be6bf453d2fb904eeec68f0cd2f81257"}}"#;
    const DICT: &[u8] = r#"{"chat":{"よろです":"잘부탁해요"}}"#.as_bytes();
    const KEY_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDEyM0EzNEMxRkU1RjEzNTUKUldSVkUxLyt3VFE2RWxFcjNsMzhMVGJIekpzTDMwVWlDZktzM3VQelZZS2o4eHJlcWpXajRCUncK";
    const KEY_B: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDkzMTkyQ0NDODhDNzhFQTgKUldTb2pzZUl6Q3daazFRRkMzSExxYnRUelVad2Y5b01lRWlEODhtYVYwTEs3S3llU2lia0p6bFAK";
    const SIG_7_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHVudHJ1c3RlZApSVVJWRTEvK3dUUTZFbDZHZWUrNTMvd3Z0ZGdVd3BPTFdCN2hwcVpWVjVrOURVZGFFa3BONGxvVVAvU3pjWTNhOGZzNTZlNHEySndQYnhmMnJpaTFOSVhBR3VYTEtpN1dDQTA9CnRydXN0ZWQgY29tbWVudDogdGltZXN0YW1wOjEJZmlsZTptZXRhZGF0YS5qc29uCXZlcnNpb246NwpIUVhKbXBYZmxzTktCKzZlSjRqQkhlWHROV1VKK3ROVDI4TXFkSGZ0N1hjZEgyVlhCLzBzbFBCZlZUdmhtSjJuQWxGMEo4ZG54NldFR2VlcmE1QWJEZz09Cg==";
    const SIG_6_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHVudHJ1c3RlZApSVVJWRTEvK3dUUTZFbGVHZmRRRGtscmI2L3dZR29vWXBEdUxPY3lidnFpRGgzcXUzbnV2cS9HZWdwYm9Uc05DYm4zck5ZMmJZcmVOK3h0OGc1SEVPeGp0VjlHMzNYSHJsZ1k9CnRydXN0ZWQgY29tbWVudDogdGltZXN0YW1wOjEJZmlsZTptZXRhZGF0YS5qc29uCXZlcnNpb246NgplQldrQzJWc1d0a2JGRHNjRTFZcXdDT0NjZ2JMQm9VR0ZrZ3FJemF0YXpQVjE4aWR6L1V5YklRRU1uWk10YThkQVd6UElJVTVZOVN5WHc5TEFlQXlCUT09Cg==";
    const SIG_7_B: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHVudHJ1c3RlZApSVVNvanNlSXpDd1prMEVyZXdHMUl0QjRnNXZNWHYxL0h3elowS0ZaZUwvVW5zQ3h2SWtCSlZpaStwOTZsSWJXajFpVGNNNm80U2dzalhFNVYxQ1hDQzdJNXdkcXhhblJ3QVE9CnRydXN0ZWQgY29tbWVudDogdGltZXN0YW1wOjEJZmlsZTptZXRhZGF0YS5qc29uCXZlcnNpb246NwozU0RhaVVnZWhqME9XVlRIdUVYU3VxbEJkbzRHZjhRZ2V4Q0JvZVJHZFlaV1Yxc1UxUjliSXQ4Rmd4VzBiU3E1cEprVG5ZQXdBRTZ0Q1hvUzJqTUZDZz09Cg==";

    #[test]
    fn metadata_signed_by_a_trusted_key_for_its_revision_is_accepted_and_read() {
        let metadata = verify_metadata(BODY, SIG_7_A, &[KEY_A], 0).unwrap();
        assert_eq!(metadata.revision, 7);
        assert_eq!(metadata.model.latest_version, "1.1.0");
        assert_eq!(metadata.model.sha256, "aa");
        assert_eq!(metadata.dictionary.version, "1.0.6");
    }

    #[test]
    fn the_same_revision_again_is_accepted_so_a_repeat_check_does_not_fail() {
        assert!(verify_metadata(BODY, SIG_7_A, &[KEY_A], 7).is_ok());
    }

    #[test]
    fn changed_bytes_are_refused() {
        let mut changed = BODY.to_vec();
        let at = changed.iter().position(|&b| b == b'1').unwrap();
        changed[at] = b'2'; // the model's version, say
        let err = verify_metadata(&changed, SIG_7_A, &[KEY_A], 0).unwrap_err();
        assert_eq!(
            err,
            MetadataError::Signature(UpdateSignatureError::NotSignedByTrustedKey)
        );
    }

    #[test]
    fn a_key_the_app_does_not_trust_is_refused() {
        let err = verify_metadata(BODY, SIG_7_B, &[KEY_A], 0).unwrap_err();
        assert_eq!(
            err,
            MetadataError::Signature(UpdateSignatureError::NotSignedByTrustedKey)
        );
        // The same signature passes when its key is one of the trusted ones (the backup key's case).
        assert!(verify_metadata(BODY, SIG_7_B, &[KEY_A, KEY_B], 0).is_ok());
    }

    #[test]
    fn a_signature_for_another_revision_is_refused() {
        // Right bytes, right key, but the signature says revision 6: an older signature put under a
        // newer file, or the other way round.
        let err = verify_metadata(BODY, SIG_6_A, &[KEY_A], 0).unwrap_err();
        assert!(
            matches!(
                err,
                MetadataError::Signature(UpdateSignatureError::VersionMismatch { .. })
            ),
            "{err:?}"
        );
    }

    #[test]
    fn an_older_signed_revision_than_one_already_accepted_is_a_rollback_and_refused() {
        let err = verify_metadata(BODY, SIG_7_A, &[KEY_A], 8).unwrap_err();
        assert_eq!(
            err,
            MetadataError::Older {
                found: 7,
                accepted: 8
            }
        );
    }

    #[test]
    fn a_file_with_no_revision_is_refused_before_anything_else() {
        for body in [
            &br#"{"model":{"latest_version":"1"},"dictionary":{"version":"1","updated_at":""}}"#[..],
            br#"{"revision":0}"#,
            br#"{"revision":"7"}"#,
        ] {
            assert_eq!(
                verify_metadata(body, SIG_7_A, &[KEY_A], 0).unwrap_err(),
                MetadataError::NoRevision,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn text_that_is_not_json_is_refused() {
        assert!(matches!(
            verify_metadata(b"<html>404</html>", SIG_7_A, &[KEY_A], 0),
            Err(MetadataError::NotJson(_))
        ));
    }

    #[test]
    fn no_signature_text_is_a_malformed_signature() {
        let err = verify_metadata(BODY, "", &[KEY_A], 0).unwrap_err();
        assert_eq!(
            err,
            MetadataError::Signature(UpdateSignatureError::MalformedSignature)
        );
    }

    #[test]
    fn a_dictionary_is_accepted_when_its_hash_is_the_signed_one() {
        let metadata = verify_metadata(BODY, SIG_7_A, &[KEY_A], 0).unwrap();
        assert_eq!(verify_dictionary(DICT, &metadata), Ok(()));
    }

    #[test]
    fn a_dictionary_with_another_hash_is_refused() {
        let metadata = verify_metadata(BODY, SIG_7_A, &[KEY_A], 0).unwrap();
        let mut other = DICT.to_vec();
        other.push(b' ');
        let err = verify_dictionary(&other, &metadata).unwrap_err();
        assert!(
            matches!(err, MetadataError::DictionaryDiffers { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn metadata_that_names_no_dictionary_hash_cannot_vouch_for_a_dictionary() {
        // The old gist's shape: no revision, no dictionary hash.
        let old: resonance_types::GistMetadata = serde_json::from_str(
            r#"{"model":{"latest_version":"1","download_url":"","release_notes":"","sha256":""},
                "dictionary":{"version":"1","updated_at":""}}"#,
        )
        .unwrap();
        assert_eq!(
            verify_dictionary(DICT, &old),
            Err(MetadataError::NoDictionaryHash)
        );
    }

    #[test]
    fn the_hash_is_compared_without_regard_to_case_or_spaces() {
        let mut metadata = verify_metadata(BODY, SIG_7_A, &[KEY_A], 0).unwrap();
        metadata.dictionary.sha256 = format!(" {} ", metadata.dictionary.sha256.to_uppercase());
        assert_eq!(verify_dictionary(DICT, &metadata), Ok(()));
    }

    #[test]
    fn a_publication_passes_when_the_metadata_and_the_dictionary_both_check_out() {
        let metadata = verify_published(BODY, SIG_7_A, DICT, &[KEY_A], 6).unwrap();
        assert_eq!(metadata.revision, 7);
    }

    #[test]
    fn a_publication_is_refused_for_a_bad_signature_a_wrong_dictionary_or_a_dictionary_that_does_not_parse(
    ) {
        assert!(matches!(
            verify_published(BODY, SIG_7_B, DICT, &[KEY_A], 0),
            Err(MetadataError::Signature(_))
        ));
        assert!(matches!(
            verify_published(BODY, SIG_7_A, b"{}", &[KEY_A], 0),
            Err(MetadataError::DictionaryDiffers { .. })
        ));
        // Signed with the right hash, but not a dictionary the app can read: build such a pair.
        let not_a_dictionary = b"[1, 2, 3]";
        let mut metadata = verify_metadata(BODY, SIG_7_A, &[KEY_A], 0).unwrap();
        let hash: String = Sha256::digest(not_a_dictionary)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        metadata.dictionary.sha256 = hash;
        assert_eq!(verify_dictionary(not_a_dictionary, &metadata), Ok(()));
        assert!(matches!(
            check_dictionary_reads(not_a_dictionary),
            Err(MetadataError::DictionaryUnreadable(_))
        ));
    }

    #[test]
    fn the_signature_is_read_from_next_to_the_metadata() {
        assert_eq!(
            signature_url("https://raw.githubusercontent.com/o/r/metadata/metadata.json"),
            "https://raw.githubusercontent.com/o/r/metadata/metadata.json.sig"
        );
        // A test run points the metadata at a mock server; the signature follows it.
        assert_eq!(
            signature_url("http://127.0.0.1:8099/metadata.json"),
            "http://127.0.0.1:8099/metadata.json.sig"
        );
    }

    #[test]
    fn metadata_with_no_signature_published_is_refused() {
        // The `.sig` was not there (404): the same refusal as a bad one, with its own reason.
        assert_eq!(
            accept_metadata(BODY, None, &[KEY_A], 0).unwrap_err(),
            MetadataError::NoSignature
        );
        assert_eq!(
            accept_metadata(BODY, Some(SIG_7_A), &[KEY_A], 0)
                .unwrap()
                .revision,
            7
        );
        assert!(matches!(
            accept_metadata(BODY, Some(SIG_7_B), &[KEY_A], 0),
            Err(MetadataError::Signature(_))
        ));
    }

    #[test]
    fn a_dictionary_is_accepted_only_when_it_is_the_named_file_and_reads() {
        let metadata = verify_metadata(BODY, SIG_7_A, &[KEY_A], 0).unwrap();
        assert_eq!(accept_dictionary(DICT, &metadata), Ok(()));
        assert!(matches!(
            accept_dictionary(b"{}", &metadata),
            Err(MetadataError::DictionaryDiffers { .. })
        ));
        // The right hash, but not a dictionary the app can read.
        let mut other = metadata.clone();
        other.dictionary.sha256 = Sha256::digest(b"[1, 2, 3]")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert!(matches!(
            accept_dictionary(b"[1, 2, 3]", &other),
            Err(MetadataError::DictionaryUnreadable(_))
        ));
    }

    #[test]
    fn a_test_key_replaces_the_app_keys_and_never_adds_to_them() {
        assert_eq!(trusted_metadata_keys(None), TRUSTED_UPDATE_KEYS);
        // With a test key only that key counts: a publication signed by the real key is refused then.
        assert_eq!(trusted_metadata_keys(Some(KEY_A)), [KEY_A]);
        assert!(!trusted_metadata_keys(Some(KEY_A)).contains(&TRUSTED_UPDATE_KEYS[0]));
    }

    #[test]
    fn the_log_line_says_what_was_refused_why_and_that_the_installed_copy_stays() {
        let line = refusal_line(&MetadataError::NoSignature);
        assert!(line.contains("model"), "{line}");
        assert!(line.contains("dictionary"), "{line}");
        assert!(
            line.contains(&MetadataError::NoSignature.to_string()),
            "{line}"
        );
        assert!(line.contains("kept"), "{line}");
        // The reason differs, so the line does.
        let older = refusal_line(&MetadataError::Older {
            found: 1,
            accepted: 2,
        });
        assert_ne!(line, older);
        assert!(older.contains("revision 1"), "{older}");
    }

    #[test]
    fn the_log_line_for_a_bad_signature_carries_the_reason_the_signature_gave() {
        let line = refusal_line(&MetadataError::Signature(
            UpdateSignatureError::NotSignedByTrustedKey,
        ));
        assert!(line.contains("signature was refused"), "{line}");
        assert!(line.contains("not signed by a trusted key"), "{line}");
    }

    #[test]
    fn the_log_line_for_a_refused_dictionary_keeps_the_installed_one_and_advises_a_retry_for_a_mismatch(
    ) {
        let differs = MetadataError::DictionaryDiffers {
            expected: "aa".into(),
            found: "bb".into(),
        };
        let line = dictionary_refusal_line(&differs);
        assert!(line.contains(&differs.to_string()), "{line}");
        assert!(line.contains("installed dictionary is kept"), "{line}");
        // A file published a moment ago can still be an old copy on some server: say to try again.
        assert!(line.contains("try again"), "{line}");
        // A dictionary that is the named file but unreadable is not a timing matter.
        let unreadable = dictionary_refusal_line(&MetadataError::DictionaryUnreadable("x".into()));
        assert!(
            unreadable.contains("installed dictionary is kept"),
            "{unreadable}"
        );
        assert!(!unreadable.contains("try again"), "{unreadable}");
    }

    #[test]
    fn the_sources_in_the_repo_are_what_the_app_reads() {
        // `metadata/` holds what gets signed and published (the workflow adds the revision and the
        // dictionary's hash): it must parse the way the app will parse the published files.
        let source = include_str!("../../../metadata/metadata.json");
        let metadata: resonance_types::GistMetadata = serde_json::from_str(source).unwrap();
        assert!(!metadata.model.download_url.is_empty());
        assert_eq!(metadata.model.sha256.len(), 64, "a SHA-256 in hex");
        assert!(!metadata.dictionary.version.is_empty());

        let dictionary = crate::text::Dictionary::from_json_str(include_str!(
            "../../../metadata/custom_dict.json"
        ))
        .unwrap();
        assert!(dictionary.len() > 100, "{} terms", dictionary.len());
        assert_eq!(
            dictionary.get("ダンジョン").map(String::as_str),
            Some("던전")
        );
    }

    #[test]
    fn every_refusal_reads_as_a_sentence_for_the_system_log() {
        let errors = [
            MetadataError::NotJson("eof".into()),
            MetadataError::NoRevision,
            MetadataError::Signature(UpdateSignatureError::NotSignedByTrustedKey),
            MetadataError::Older {
                found: 3,
                accepted: 5,
            },
            MetadataError::NoDictionaryHash,
            MetadataError::DictionaryDiffers {
                expected: "aa".into(),
                found: "bb".into(),
            },
            MetadataError::DictionaryUnreadable("not an object".into()),
        ];
        for error in errors {
            let text = error.to_string();
            assert!(text.len() > 20 && !text.contains("{:?}"), "{text}");
        }
    }
}
