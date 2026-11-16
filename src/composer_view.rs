//! The pure parts of the chat composer: the live counter, when Send is enabled,
//! which channel a tab preselects, when Enter sends, and the request. The
//! component (`components/chat_composer.rs`) only wires signals and events.

use crate::chat_view::Tab;
use resonance_types::ipc::SendChat;
use resonance_types::{prepare_text, SendChatChannel, SendTextError, MAX_TEXT_UTF16};

/// The counter's tooltip: the limit is the game's, in UTF-16 units.
pub const COUNTER_TITLE: &str = "글자 수는 UTF-16 기준이라 이모지는 2로 셉니다.";

/// UTF-16 units of the prepared text (what the backend sends); 0 when empty.
pub fn counter_len(draft: &str) -> usize {
    match prepare_text(draft) {
        Ok(p) => p.encode_utf16().count(),
        Err(SendTextError::TooLong { len }) => len,
        Err(_) => 0,
    }
}

/// The counter's text, e.g. `0/140`.
pub fn counter_text(draft: &str) -> String {
    format!("{}/{MAX_TEXT_UTF16}", counter_len(draft))
}

/// Whether the draft is over the limit.
pub fn counter_over(draft: &str) -> bool {
    counter_len(draft) > MAX_TEXT_UTF16
}

/// Whether Send is enabled: something to send, within the limit, none in flight.
pub fn can_send(draft: &str, sending: bool) -> bool {
    !sending && prepare_text(draft).is_ok()
}

/// The channel a tab preselects; `None` for tabs without one (the picker stays).
pub fn preselect(tab_label: &str) -> Option<SendChatChannel> {
    match Tab::parse(tab_label)? {
        Tab::Channel(c) => SendChatChannel::from_channel(c),
        _ => None,
    }
}

/// Whether a key press sends: a plain Enter, not Shift+Enter, and not one an
/// input method handles (`is_composing`, or key code 229 as Chromium reports it).
pub fn enter_sends(key: &str, shift: bool, composing: bool, key_code: u32) -> bool {
    key == "Enter" && !shift && !composing && key_code != 229
}

/// The request: the draft exactly as typed, no prefix (the command prepares it).
pub fn send_request(channel: SendChatChannel, draft: &str) -> SendChat {
    SendChat {
        channel,
        text: draft.to_string(),
    }
}

/// The picker's options as (value, label), in tab order.
pub fn channel_options() -> Vec<(&'static str, &'static str)> {
    SendChatChannel::ALL
        .iter()
        .map(|c| (c.as_str(), c.label()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_view::{Tab, ALL_TAB, CUSTOM_TAB, SYSTEM_TAB};
    use crate::ui_types::Channel;
    use resonance_types::SendChatChannel;

    fn a(n: usize) -> String {
        "a".repeat(n)
    }

    #[test]
    fn the_counter_counts_the_prepared_text() {
        assert_eq!(counter_text(""), "0/140");
        assert_eq!(counter_text("   "), "0/140");
        assert_eq!(counter_text("hi"), "2/140");
        assert_eq!(counter_text("  hi  "), "2/140");
        assert_eq!(counter_text(&a(140)), "140/140");
        assert!(!counter_over(&a(140)));
        assert_eq!(counter_text(&a(141)), "141/140");
        assert!(counter_over(&a(141)));
        assert_eq!(counter_text(&"😀".repeat(70)), "140/140");
        assert_eq!(counter_text(&"😀".repeat(71)), "142/140");
        assert!(counter_over(&"😀".repeat(71)));
        assert_eq!(counter_text(&format!("{}\n{}", a(70), a(70))), "141/140");
        assert_eq!(
            COUNTER_TITLE,
            "글자 수는 UTF-16 기준이라 이모지는 2로 셉니다."
        );
    }

    #[test]
    fn send_is_enabled_for_a_sendable_idle_draft() {
        assert!(!can_send("", false));
        assert!(!can_send("   ", false));
        assert!(can_send("hi", false));
        assert!(!can_send("hi", true));
        assert!(!can_send(&a(141), false));
        assert!(can_send(&a(140), false));
        assert!(!can_send(&"😀".repeat(71), false));
    }

    #[test]
    fn a_channel_tab_preselects_its_channel() {
        for c in SendChatChannel::ALL {
            let channel = Channel::ALL
                .into_iter()
                .find(|ch| SendChatChannel::from_channel(*ch) == Some(c))
                .unwrap();
            assert_eq!(preselect(Tab::Channel(channel).label()), Some(c));
        }
        assert_eq!(preselect("월드"), Some(SendChatChannel::World));
        assert_eq!(preselect(Tab::Channel(Channel::Beginner).label()), None);
        assert_eq!(preselect(ALL_TAB), None);
        assert_eq!(preselect(CUSTOM_TAB), None);
        assert_eq!(preselect(SYSTEM_TAB), None);
        assert_eq!(preselect("???"), None);
    }

    #[test]
    fn only_a_plain_enter_sends() {
        assert!(enter_sends("Enter", false, false, 13));
        assert!(!enter_sends("Enter", true, false, 13));
        assert!(!enter_sends("Enter", false, true, 13));
        assert!(!enter_sends("Enter", false, false, 229));
        assert!(!enter_sends("a", false, false, 65));
        assert!(!enter_sends("Escape", false, false, 27));
    }

    #[test]
    fn the_request_carries_the_draft_as_typed() {
        let req = send_request(SendChatChannel::Guild, "  hi\n");
        assert_eq!(req.channel, SendChatChannel::Guild);
        assert_eq!(req.text, "  hi\n");
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            serde_json::json!({"channel": "guild", "text": "  hi\n"})
        );
    }

    #[test]
    fn the_picker_lists_every_channel() {
        assert_eq!(
            channel_options(),
            vec![
                ("world", "월드"),
                ("guild", "길드"),
                ("party", "파티"),
                ("say", "로컬")
            ]
        );
    }
}
