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

/// What every progress label of the in-game module download starts with (the backend's
/// `ingame_fetch::MODULE_LABEL_PREFIX`; the ui does not depend on the core crate).
const MODULE_LABEL_PREFIX: &str = "게임 내 전송 모듈";

/// The composer's line for a `download-progress` event: the module download below 100 %;
/// `None` for any other download or when it is done.
pub fn module_line(label: &str, percent: u8) -> Option<String> {
    if !label.starts_with(MODULE_LABEL_PREFIX) || percent >= 100 {
        return None;
    }
    Some(format!("게임 내 전송 모듈을 내려받는 중... {percent}%"))
}

/// The channel a tab preselects; `None` for tabs without one (the picker stays).
pub fn preselect(tab_label: &str) -> Option<SendChatChannel> {
    match Tab::parse(tab_label)? {
        Tab::Channel(c) => SendChatChannel::from_channel(c),
        _ => None,
    }
}

/// The channel a saved wire name (`send_channel` in the config) stands for; an exact match, else
/// World (a missing, hand-edited or unknown value).
pub fn stored_channel(s: &str) -> SendChatChannel {
    SendChatChannel::ALL
        .into_iter()
        .find(|c| c.as_str() == s)
        .unwrap_or(SendChatChannel::ALL[0])
}

/// What the preselect Effect does: the channel to switch to, or `None`. Not on the first run
/// (the saved channel wins on start), and not when the tab's channel is already the stored one
/// (no change, no save).
pub fn tab_pick(first_run: bool, tab_label: &str, stored: &str) -> Option<SendChatChannel> {
    if first_run {
        return None;
    }
    preselect(tab_label).filter(|c| *c != stored_channel(stored))
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
        client: None,
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
    use resonance_types::ipc::{GameClient, CLIENT_GONE_LINE};
    use resonance_types::SendChatChannel;

    fn a(n: usize) -> String {
        "a".repeat(n)
    }

    #[test]
    fn the_module_download_shows_a_line_below_100_percent_only() {
        let label = "게임 내 전송 모듈 내려받는 중...";
        assert_eq!(
            module_line(label, 40).as_deref(),
            Some("게임 내 전송 모듈을 내려받는 중... 40%")
        );
        assert_eq!(
            module_line(label, 0).as_deref(),
            Some("게임 내 전송 모듈을 내려받는 중... 0%")
        );
        assert_eq!(module_line(label, 100), None);
        assert_eq!(module_line("앱 업데이트", 50), None);
        assert_eq!(module_line("AI 모델 다운로드", 10), None);
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
        let req = send_request(SendChatChannel::Guild, "  hi\n", None);
        assert_eq!(req.channel, SendChatChannel::Guild);
        assert_eq!(req.text, "  hi\n");
        assert_eq!(req.client, None);
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            serde_json::json!({"channel": "guild", "text": "  hi\n", "client": null})
        );
        assert_eq!(
            send_request(SendChatChannel::World, "hi", Some(7)).client,
            Some(7)
        );
    }

    fn c(target: u32, name: &str) -> GameClient {
        GameClient {
            target,
            name: name.into(),
        }
    }

    #[test]
    fn the_picker_shows_only_with_two_or_more_clients() {
        assert!(!show_picker(&[]));
        assert!(!show_picker(&[c(7, "Aria")]));
        assert!(show_picker(&[c(7, "Aria"), c(42, "Bob")]));
    }

    #[test]
    fn options_are_nicknames_with_an_opaque_value() {
        let opts = client_options(&[c(7, "Aria"), c(42, "Bob")]);
        let labels: Vec<_> = opts.iter().map(|(_, l)| l.as_str()).collect();
        let values: Vec<_> = opts.iter().map(|(v, _)| v.as_str()).collect();
        assert_eq!(labels, ["Aria", "Bob"]);
        assert_eq!(values, ["7", "42"]);
        for (v, l) in &opts {
            assert!(l != v && !l.contains(v.as_str()));
        }
        assert_eq!(client_from_value("42"), Some(42));
        for bad in ["", "x", "-1"] {
            assert_eq!(client_from_value(bad), None);
        }
    }

    #[test]
    fn the_send_plan_never_redirects_a_gone_choice() {
        let two = [c(7, "Aria"), c(42, "Bob")];
        assert_eq!(send_plan(&two, Some(42)), SendPlan::Send(Some(42)));
        assert_eq!(send_plan(&two, None), SendPlan::Send(None));
        assert_eq!(send_plan(&[c(7, "Aria")], None), SendPlan::Send(None));
        assert_eq!(send_plan(&[c(7, "Aria")], Some(7)), SendPlan::Send(None));
        assert_eq!(
            send_plan(&[c(7, "Aria"), c(50, "Cy")], Some(42)),
            SendPlan::Gone
        );
        assert_eq!(send_plan(&[c(7, "Aria")], Some(42)), SendPlan::Gone);
        assert_eq!(send_plan(&[], None), SendPlan::Send(None));
    }

    #[test]
    fn only_the_gone_sentence_clears_the_choice() {
        assert!(clears_choice(CLIENT_GONE_LINE));
        assert!(!clears_choice(
            "게임이 둘 이상 연결되어 있습니다. 보낼 게임을 골라 주세요."
        ));
        assert!(!clears_choice(""));
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

    #[test]
    fn the_stored_channel_is_restored_and_anything_else_is_world() {
        for c in SendChatChannel::ALL {
            assert_eq!(stored_channel(c.as_str()), c);
        }
        assert_eq!(stored_channel("guild"), SendChatChannel::Guild);
        for odd in ["", "GUILD", "whisper", "beginner", "길드"] {
            assert_eq!(stored_channel(odd), SendChatChannel::World, "{odd:?}");
        }
    }

    #[test]
    fn a_tab_change_picks_a_channel_only_when_it_differs_and_not_on_the_first_run() {
        use SendChatChannel::{Guild, Say};
        assert_eq!(tab_pick(true, "길드", "world"), None);
        assert_eq!(tab_pick(false, "길드", "world"), Some(Guild));
        assert_eq!(tab_pick(false, "길드", "guild"), None);
        assert_eq!(tab_pick(false, "로컬", "world"), Some(Say));
        for label in [ALL_TAB, CUSTOM_TAB, SYSTEM_TAB, "신입", "???"] {
            assert_eq!(tab_pick(false, label, "party"), None, "{label}");
        }
        assert_eq!(tab_pick(false, "신입", "guild"), None);
    }
}
