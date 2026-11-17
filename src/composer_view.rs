//! The pure parts of the chat composer: the live counter, when Send is enabled,
//! which channel a tab preselects, when Enter sends, and the request. The
//! component (`components/chat_composer.rs`) only wires signals and events.

use crate::chat_view::Tab;
use resonance_types::ipc::{GameClient, SendChat, SendVerdict, CLIENT_GONE_LINE};
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

/// The request: the draft exactly as typed, no prefix (the command prepares it), and the chosen
/// client (a `GameClient.target`, handed back unchanged).
pub fn send_request(channel: SendChatChannel, draft: &str, client: Option<u32>) -> SendChat {
    SendChat {
        channel,
        text: draft.to_string(),
        client,
    }
}

/// Whether the client picker shows: only with two or more clients.
pub fn show_picker(clients: &[GameClient]) -> bool {
    clients.len() >= 2
}

/// The picker's options as (value, label) in list order: the label is the nickname only; the
/// value is the opaque target as text, never displayed.
pub fn client_options(clients: &[GameClient]) -> Vec<(String, String)> {
    clients
        .iter()
        .map(|c| (c.target.to_string(), c.name.clone()))
        .collect()
}

/// The client an `<option value>` stands for; `None` for the placeholder or anything else.
pub fn client_from_value(value: &str) -> Option<u32> {
    value.parse().ok()
}

/// What a send does with the list just read and the chosen client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPlan {
    /// Call `send_chat` with this client.
    Send(Option<u32>),
    /// The chosen client is no longer listed: ask to choose again, send nothing.
    Gone,
}

/// A chosen client that is not listed is `Gone` (never sent to a survivor); fewer than two
/// clients send with none; otherwise the choice goes as it is (a `None` gets the backend's
/// "choose a client" sentence).
pub fn send_plan(clients: &[GameClient], chosen: Option<u32>) -> SendPlan {
    match chosen {
        Some(p) if !clients.iter().any(|c| c.target == p) => SendPlan::Gone,
        _ if clients.len() < 2 => SendPlan::Send(None),
        _ => SendPlan::Send(chosen),
    }
}

/// Whether a failed send's sentence means the chosen client is gone, so the choice resets.
pub fn clears_choice(error: &str) -> bool {
    error == CLIENT_GONE_LINE
}

/// The status-line sentence for what the server said; empty when it took the message.
pub fn verdict_line(verdict: &SendVerdict) -> String {
    match verdict {
        SendVerdict::Sent => String::new(),
        SendVerdict::TooFrequent => "너무 자주 보냈습니다. 잠시 후 다시 보내 주세요.".to_string(),
        SendVerdict::Cooldown { remaining_secs } => countdown_line(*remaining_secs),
        SendVerdict::NotConfirmed => "보냈지만 서버의 확인을 받지 못했습니다.".to_string(),
        // The code is for the log, not the user.
        SendVerdict::Refused { .. } => "서버가 메시지를 받지 않았습니다.".to_string(),
    }
}

/// Whether the draft is cleared: the message went out (the server's word or not).
pub fn clears_draft(verdict: &SendVerdict) -> bool {
    matches!(verdict, SendVerdict::Sent | SendVerdict::NotConfirmed)
}

/// A cooldown the server answered for one client and channel, until a wall-clock time (ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CooldownBlock {
    pub client: Option<u32>,
    pub channel: SendChatChannel,
    pub until_ms: u64,
}

/// The block a verdict makes; only a `Cooldown` of at least one second does. The end is
/// clamped to 600 s (a placeholder bound against a wrong clock).
pub fn block_from(
    verdict: &SendVerdict,
    client: Option<u32>,
    channel: SendChatChannel,
    arrived_ms: u64,
) -> Option<CooldownBlock> {
    match verdict {
        SendVerdict::Cooldown { remaining_secs } if *remaining_secs > 0 => Some(CooldownBlock {
            client,
            channel,
            until_ms: arrived_ms + u64::from((*remaining_secs).min(600)) * 1000,
        }),
        _ => None,
    }
}

/// Adds a block, replacing the one for the same client and channel.
pub fn add_block(blocks: &mut Vec<CooldownBlock>, new: CooldownBlock) {
    match blocks
        .iter_mut()
        .find(|b| b.client == new.client && b.channel == new.channel)
    {
        Some(old) => *old = new,
        None => blocks.push(new),
    }
}

/// Whole seconds left (rounded up) for exactly this client and channel; 0 when none.
pub fn seconds_left(
    blocks: &[CooldownBlock],
    client: Option<u32>,
    channel: SendChatChannel,
    now_ms: u64,
) -> u32 {
    blocks
        .iter()
        .find(|b| b.client == client && b.channel == channel)
        .map_or(0, |b| {
            let left = b.until_ms.saturating_sub(now_ms);
            u32::try_from(left.div_ceil(1000)).unwrap_or(u32::MAX)
        })
}

/// Whether any block is still running (the timer runs exactly then).
pub fn counting(blocks: &[CooldownBlock], now_ms: u64) -> bool {
    blocks.iter().any(|b| b.until_ms > now_ms)
}

/// Drops the blocks that have ended.
pub fn prune(blocks: &mut Vec<CooldownBlock>, now_ms: u64) {
    blocks.retain(|b| b.until_ms > now_ms);
}

/// The client the next send goes with; a stale choice (`Gone`) has none, so no block applies.
pub fn send_target(clients: &[GameClient], chosen: Option<u32>) -> Option<u32> {
    match send_plan(clients, chosen) {
        SendPlan::Send(c) => c,
        SendPlan::Gone => None,
    }
}

/// Whether Send is allowed: `can_send` and no cooldown left.
pub fn can_send_now(draft: &str, sending: bool, left_secs: u32) -> bool {
    can_send(draft, sending) && left_secs == 0
}

/// The cooldown sentence for the seconds left (a placeholder wording).
pub fn countdown_line(left_secs: u32) -> String {
    format!("아직 보낼 수 없습니다. {left_secs}초 뒤에 다시 보내 주세요.")
}

/// How the status line is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Error,
    Notice,
}

/// The status line: a sentence and its tone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub tone: Tone,
    pub text: String,
}

/// The line a verdict stores; a `Cooldown` is shown live from its block, not stored.
pub fn status_of(verdict: &SendVerdict) -> Option<Status> {
    let tone = match verdict {
        SendVerdict::Sent | SendVerdict::Cooldown { .. } => return None,
        SendVerdict::NotConfirmed => Tone::Notice,
        SendVerdict::TooFrequent | SendVerdict::Refused { .. } => Tone::Error,
    };
    Some(Status {
        tone,
        text: verdict_line(verdict),
    })
}

/// What the line shows: the live countdown while one runs, else the stored status.
pub fn shown_status(stored: Option<&Status>, left_secs: u32) -> Option<Status> {
    if left_secs > 0 {
        return Some(Status {
            tone: Tone::Error,
            text: countdown_line(left_secs),
        });
    }
    stored.cloned()
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
    fn a_channel_tab_gives_the_picker_its_channel_whatever_it_showed() {
        // The picker's value after a tab change: the tab's channel, else what it held.
        fn after(stored: &str, tab: &str) -> SendChatChannel {
            tab_pick(false, tab, stored).unwrap_or(stored_channel(stored))
        }
        for from in SendChatChannel::ALL {
            for c in SendChatChannel::ALL {
                assert_eq!(after(from.as_str(), c.label()), c, "{from:?} -> {c:?}");
            }
            for tab in [
                ALL_TAB,
                CUSTOM_TAB,
                SYSTEM_TAB,
                Tab::Channel(Channel::Beginner).label(),
            ] {
                assert_eq!(after(from.as_str(), tab), from, "{from:?} on {tab}");
            }
        }
        assert_eq!(after("party", "길드"), SendChatChannel::Guild);
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
    #[test]
    fn each_verdict_has_its_own_sentence() {
        assert_eq!(verdict_line(&SendVerdict::Sent), "");
        let cooldown = verdict_line(&SendVerdict::Cooldown { remaining_secs: 3 });
        assert_eq!(
            cooldown,
            "아직 보낼 수 없습니다. 3초 뒤에 다시 보내 주세요."
        );
        assert!(cooldown.contains("3초"));
        let lines = [
            verdict_line(&SendVerdict::Sent),
            cooldown,
            verdict_line(&SendVerdict::TooFrequent),
            verdict_line(&SendVerdict::NotConfirmed),
            verdict_line(&SendVerdict::Refused { code: 9999 }),
        ];
        assert_eq!(lines[2], "너무 자주 보냈습니다. 잠시 후 다시 보내 주세요.");
        assert_eq!(lines[3], "보냈지만 서버의 확인을 받지 못했습니다.");
        assert_eq!(lines[4], "서버가 메시지를 받지 않았습니다.");
        assert!(!lines[4].contains("9999"));
        for (i, a) in lines.iter().enumerate() {
            for b in &lines[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn the_draft_is_cleared_only_when_the_message_went_out() {
        assert!(clears_draft(&SendVerdict::Sent));
        assert!(clears_draft(&SendVerdict::NotConfirmed));
        assert!(!clears_draft(&SendVerdict::TooFrequent));
        assert!(!clears_draft(&SendVerdict::Cooldown { remaining_secs: 3 }));
        assert!(!clears_draft(&SendVerdict::Refused { code: 1 }));
    }

    const W: SendChatChannel = SendChatChannel::World;
    const P: SendChatChannel = SendChatChannel::Party;
    const A: Option<u32> = Some(7);
    const B: Option<u32> = Some(42);
    const T: u64 = 1_000_000;

    fn cd(n: u32) -> SendVerdict {
        SendVerdict::Cooldown { remaining_secs: n }
    }

    fn block(client: Option<u32>, channel: SendChatChannel, secs: u64) -> CooldownBlock {
        CooldownBlock {
            client,
            channel,
            until_ms: T + secs * 1000,
        }
    }

    #[test]
    fn only_a_cooldown_makes_a_block_and_its_end_is_clamped() {
        assert_eq!(block_from(&cd(3), A, W, T), Some(block(A, W, 3)));
        assert_eq!(block_from(&cd(0), A, W, T), None);
        assert_eq!(
            block_from(&cd(5000), A, W, T).unwrap().until_ms,
            T + 600_000
        );
        for v in [
            SendVerdict::Sent,
            SendVerdict::TooFrequent,
            SendVerdict::NotConfirmed,
            SendVerdict::Refused { code: 9999 },
        ] {
            assert_eq!(block_from(&v, A, W, T), None, "{v:?}");
        }
    }

    #[test]
    fn seconds_left_rounds_up_and_never_underflows() {
        let b = [block(A, W, 3)];
        for (now, want) in [
            (T, 3),
            (T + 1, 3),
            (T + 1000, 2),
            (T + 2000, 1),
            (T + 2999, 1),
            (T + 3000, 0),
            (T + 9000, 0),
            (T - 5000, 8),
        ] {
            assert_eq!(seconds_left(&b, A, W, now), want, "now={now}");
        }
        assert_eq!(seconds_left(&[], A, W, T), 0);
    }

    #[test]
    fn a_block_applies_to_its_own_client_and_channel_only() {
        let b = [block(A, W, 3)];
        assert_eq!(seconds_left(&b, A, P, T), 0);
        assert_eq!(seconds_left(&b, B, W, T), 0);
        assert_eq!(seconds_left(&b, None, W, T), 0);
        assert_eq!(seconds_left(&b, A, W, T), 3);
        let two = [block(A, W, 3), block(B, P, 10)];
        assert_eq!(seconds_left(&two, A, W, T), 3);
        assert_eq!(seconds_left(&two, B, P, T), 10);
    }

    #[test]
    fn add_block_keeps_one_entry_per_key_and_prune_drops_the_ended() {
        let mut v = vec![block(A, W, 3)];
        add_block(&mut v, block(A, W, 8));
        assert_eq!(v, vec![block(A, W, 8)]);
        add_block(&mut v, block(A, P, 3));
        assert_eq!(v.len(), 2);
        let mut v = vec![block(A, W, 3), block(B, P, 10)];
        prune(&mut v, T + 3000);
        assert_eq!(v, vec![block(B, P, 10)]);
        prune(&mut v, T + 10_000);
        assert!(v.is_empty());
    }

    #[test]
    fn the_timer_runs_exactly_while_a_block_runs() {
        assert!(!counting(&[], T));
        let b = [block(A, W, 3)];
        assert!(counting(&b, T));
        assert!(counting(&b, T + 2999));
        assert!(!counting(&b, T + 3000));
        let two = [block(A, W, 3), block(B, P, 10)];
        assert!(counting(&two, T + 5000));
    }

    #[test]
    fn the_send_target_and_the_send_gate() {
        let two = [c(7, "Aria"), c(42, "Bob")];
        assert_eq!(send_target(&two, Some(42)), B);
        assert_eq!(send_target(&two, None), None);
        assert_eq!(send_target(&[c(7, "Aria")], None), None);
        assert_eq!(send_target(&two, Some(9)), None);
        assert_eq!(
            countdown_line(3),
            "아직 보낼 수 없습니다. 3초 뒤에 다시 보내 주세요."
        );
        assert_eq!(countdown_line(3), verdict_line(&cd(3)));
        assert!(can_send_now("hi", false, 0));
        assert!(!can_send_now("hi", false, 2));
        assert!(!can_send_now("hi", true, 0));
        assert!(!can_send_now("", false, 0));
    }

    fn st(tone: Tone, text: &str) -> Status {
        Status {
            tone,
            text: text.to_string(),
        }
    }

    #[test]
    fn a_verdict_stores_its_line_and_a_cooldown_none() {
        assert_eq!(status_of(&SendVerdict::Sent), None);
        assert_eq!(status_of(&cd(3)), None);
        assert_eq!(
            status_of(&SendVerdict::NotConfirmed),
            Some(st(Tone::Notice, "보냈지만 서버의 확인을 받지 못했습니다."))
        );
        for v in [
            SendVerdict::TooFrequent,
            SendVerdict::Refused { code: 9999 },
        ] {
            assert_eq!(status_of(&v), Some(st(Tone::Error, &verdict_line(&v))));
        }
        assert!(!status_of(&SendVerdict::Refused { code: 9999 })
            .unwrap()
            .text
            .contains("9999"));
    }

    #[test]
    fn the_countdown_wins_the_line_and_is_never_a_notice() {
        let notice = st(Tone::Notice, "n");
        let error = st(Tone::Error, "e");
        assert_eq!(shown_status(None, 0), None);
        assert_eq!(
            shown_status(None, 3),
            Some(st(Tone::Error, &countdown_line(3)))
        );
        assert_eq!(shown_status(Some(&notice), 0), Some(notice.clone()));
        assert_eq!(
            shown_status(Some(&error), 2),
            Some(st(Tone::Error, &countdown_line(2)))
        );
        assert_eq!(
            shown_status(Some(&notice), 1),
            Some(st(Tone::Error, &countdown_line(1)))
        );
    }

    #[test]
    fn the_draft_and_the_line_for_every_verdict() {
        // (verdict, clears the draft, stored tone)
        let table = [
            (SendVerdict::Sent, true, None),
            (SendVerdict::NotConfirmed, true, Some(Tone::Notice)),
            (SendVerdict::TooFrequent, false, Some(Tone::Error)),
            (cd(3), false, None),
            (SendVerdict::Refused { code: 1 }, false, Some(Tone::Error)),
        ];
        for (v, clears, tone) in table {
            assert_eq!(clears_draft(&v), clears, "{v:?}");
            assert_eq!(status_of(&v).map(|s| s.tone), tone, "{v:?}");
        }
    }
}
