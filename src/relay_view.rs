//! What the Settings page says about the guild relay (`RelayStatus`). Pure, so it is tested on
//! the host.

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_types::RelayStatus;

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
