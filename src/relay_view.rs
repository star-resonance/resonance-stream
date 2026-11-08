//! What the Settings page says about the guild relay (`RelayStatus`). Pure, so it is tested on
//! the host.

use resonance_types::RelayStatus;

/// How the status line is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Nothing to do (off).
    Muted,
    /// On its way.
    Info,
    /// Working.
    Success,
    /// Not working yet, and nothing is wrong with the member's settings.
    Warning,
    /// Not working until something changes.
    Error,
}

impl Tone {
    /// The daisyUI text colour class.
    pub fn class(self) -> &'static str {
        match self {
            Tone::Muted => "text-base-content/60",
            Tone::Info => "text-info",
            Tone::Success => "text-success",
            Tone::Warning => "text-warning",
            Tone::Error => "text-error",
        }
    }
}

/// The sentence and tone for `status`. `broker_url` is the setting as saved: empty means the
/// built-in default broker.
pub fn status_line(status: RelayStatus, broker_url: &str) -> (&'static str, Tone) {
    match status {
        RelayStatus::Off => ("꺼져 있습니다.", Tone::Muted),
        RelayStatus::NoBroker if broker_url.trim().is_empty() => (
            "기본 브로커가 아직 준비되지 않았습니다. 브로커 주소를 입력하세요.",
            Tone::Warning,
        ),
        RelayStatus::NoBroker => (
            "브로커 주소가 올바르지 않습니다. https:// 로 시작해야 합니다.",
            Tone::Error,
        ),
        RelayStatus::WaitingForGuild => (
            "길드 정보를 기다리는 중입니다. 게임에 접속해 길드에 가입되어 있어야 합니다.",
            Tone::Info,
        ),
        RelayStatus::Checking => ("브로커에 길드 등록 여부를 확인하는 중입니다.", Tone::Info),
        RelayStatus::NotEnrolled => (
            "이 브로커는 현재 길드를 받지 않습니다. 10분 후 다시 확인합니다.",
            Tone::Error,
        ),
        RelayStatus::Relaying => ("길드 채팅을 디스코드로 보내는 중입니다.", Tone::Success),
        RelayStatus::Rejected => (
            "브로커가 마지막 메시지를 거부했습니다. 앱 업데이트가 필요할 수 있습니다.",
            Tone::Error,
        ),
        RelayStatus::Retrying => (
            "브로커에 연결할 수 없습니다. 잠시 후 다시 시도합니다.",
            Tone::Warning,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [RelayStatus; 8] = [
        RelayStatus::Off,
        RelayStatus::NoBroker,
        RelayStatus::WaitingForGuild,
        RelayStatus::Checking,
        RelayStatus::NotEnrolled,
        RelayStatus::Relaying,
        RelayStatus::Rejected,
        RelayStatus::Retrying,
    ];

    #[test]
    fn every_status_has_its_own_sentence() {
        let sentences: std::collections::HashSet<_> = ALL
            .iter()
            .map(|&s| status_line(s, "https://broker.test").0)
            .collect();
        assert_eq!(sentences.len(), ALL.len());
        assert!(sentences.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn no_broker_says_why_an_empty_field_means_the_default_is_not_ready() {
        let (empty, empty_tone) = status_line(RelayStatus::NoBroker, "");
        let (typed, typed_tone) = status_line(RelayStatus::NoBroker, "http://broker.test");
        assert!(empty.contains("기본 브로커"), "{empty}");
        assert!(typed.contains("https://"), "{typed}");
        assert_eq!(empty_tone, Tone::Warning);
        assert_eq!(typed_tone, Tone::Error);
        // Whitespace only is as good as empty: the app trims nothing, but it is not an address.
        assert_eq!(status_line(RelayStatus::NoBroker, "  ").0, empty);
    }

    #[test]
    fn the_tone_matches_what_the_member_has_to_do() {
        assert_eq!(status_line(RelayStatus::Off, "").1, Tone::Muted);
        assert_eq!(status_line(RelayStatus::Relaying, "").1, Tone::Success);
        assert_eq!(status_line(RelayStatus::NotEnrolled, "").1, Tone::Error);
        assert_eq!(status_line(RelayStatus::Rejected, "").1, Tone::Error);
        assert_eq!(status_line(RelayStatus::Retrying, "").1, Tone::Warning);
        assert_eq!(status_line(RelayStatus::Checking, "").1, Tone::Info);
    }
}
