//! What the chat composer may send: the channel, and the rules for its text.
//! Pure and std-only, so the ui compiles it to wasm for its live counter.

use crate::Channel;
use serde::{Deserialize, Serialize};

/// A channel the app can send to. On the wire it is lower-case (`"say"`,
/// `"world"`, `"party"`, `"guild"`), unlike [`Channel`]'s upper-case. There is
/// no whisper and no beginner channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SendChatChannel {
    /// The game's current (Local) channel.
    Say,
    World,
    Party,
    Guild,
}

impl SendChatChannel {
    /// Every sendable channel, in the order the chat tabs list them.
    pub const ALL: [SendChatChannel; 4] = [
        SendChatChannel::World,
        SendChatChannel::Guild,
        SendChatChannel::Party,
        SendChatChannel::Say,
    ];

    /// The wire string (lower-case).
    pub fn as_str(self) -> &'static str {
        match self {
            SendChatChannel::Say => "say",
            SendChatChannel::World => "world",
            SendChatChannel::Party => "party",
            SendChatChannel::Guild => "guild",
        }
    }

    /// The chat tab's label (Korean); `Say` is the Local tab's.
    pub fn label(self) -> &'static str {
        match self {
            SendChatChannel::Say => Channel::Local.label(),
            SendChatChannel::World => Channel::World.label(),
            SendChatChannel::Party => Channel::Party.label(),
            SendChatChannel::Guild => Channel::Guild.label(),
        }
    }

    /// The channel to send to when `channel`'s tab is open; `None` for the
    /// beginner channel, which cannot be sent to.
    pub fn from_channel(channel: Channel) -> Option<Self> {
        match channel {
            Channel::World => Some(SendChatChannel::World),
            Channel::Guild => Some(SendChatChannel::Guild),
            Channel::Party => Some(SendChatChannel::Party),
            Channel::Local => Some(SendChatChannel::Say),
            Channel::Beginner => None,
        }
    }
}

/// The longest message the game's chat takes, in UTF-16 code units.
pub const MAX_TEXT_UTF16: usize = 140;

/// Why a message is not sent. Its `Display` is the one Korean sentence the ui
/// shows (the `send_chat` command's `Err(String)`). One type for the text rules
/// here and the spacing rule in `resonance-core`, so a caller can use `?` on both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendTextError {
    /// Nothing left after trimming.
    Empty,
    /// More than [`MAX_TEXT_UTF16`] units after preparation; `len` is the count.
    TooLong { len: usize },
    /// The last successful send was too recent; `wait_ms` is the time left.
    TooSoon { wait_ms: u64 },
}

impl std::fmt::Display for SendTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendTextError::Empty => write!(f, "보낼 내용이 없습니다."),
            SendTextError::TooLong { len } => write!(
                f,
                "메시지가 너무 깁니다. {MAX_TEXT_UTF16}자 이하로 줄여 주세요 (현재 {len}자)."
            ),
            SendTextError::TooSoon { wait_ms } => {
                let tenths = wait_ms.div_ceil(100);
                write!(
                    f,
                    "너무 빨리 보내고 있습니다. {}.{}초 뒤에 다시 시도하세요.",
                    tenths / 10,
                    tenths % 10
                )
            }
        }
    }
}

impl std::error::Error for SendTextError {}

/// The characters the DLL's second check rejects: Unicode Cc (which includes
/// CR and LF) plus U+2028 and U+2029, which are not Cc.
fn is_replaced(c: char) -> bool {
    c.is_control() || c == '\u{2028}' || c == '\u{2029}'
}

/// Prepares `text` for sending: turns each control character (Unicode Cc,
/// the class the DLL's second check rejects: CR, LF, TAB, NUL, ...) and each
/// U+2028/U+2029 into one space (no collapsing, so `"a\r\nb"` gives
/// `"a  b"`), then trims (Unicode white space), then checks. The replacement
/// runs before the trim, so a lone NUL is [`SendTextError::Empty`] and never
/// a blank message. Empty after the trim is [`SendTextError::Empty`]; more
/// than [`MAX_TEXT_UTF16`] UTF-16 units after the replacement is
/// [`SendTextError::TooLong`].
pub fn prepare_text(text: &str) -> Result<String, SendTextError> {
    let replaced: String = text
        .chars()
        .map(|c| if is_replaced(c) { ' ' } else { c })
        .collect();
    let prepared = replaced.trim();
    if prepared.is_empty() {
        return Err(SendTextError::Empty);
    }
    let len = prepared.encode_utf16().count();
    if len > MAX_TEXT_UTF16 {
        return Err(SendTextError::TooLong { len });
    }
    Ok(prepared.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Channel;

    #[test]
    fn channel_serializes_lower_case_and_round_trips() {
        for (c, s) in [
            (SendChatChannel::Say, "say"),
            (SendChatChannel::World, "world"),
            (SendChatChannel::Party, "party"),
            (SendChatChannel::Guild, "guild"),
        ] {
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{s}\""));
            assert_eq!(serde_json::from_str::<SendChatChannel>(&json).unwrap(), c);
            assert_eq!(c.as_str(), s);
        }
        assert!(serde_json::from_str::<SendChatChannel>("\"WORLD\"").is_err());
        assert!(serde_json::from_str::<SendChatChannel>("\"whisper\"").is_err());
    }

    #[test]
    fn all_is_in_tab_order() {
        assert_eq!(
            SendChatChannel::ALL,
            [
                SendChatChannel::World,
                SendChatChannel::Guild,
                SendChatChannel::Party,
                SendChatChannel::Say
            ]
        );
    }

    #[test]
    fn labels_are_the_tab_labels() {
        let labels: Vec<_> = SendChatChannel::ALL.iter().map(|c| c.label()).collect();
        assert_eq!(labels, ["월드", "길드", "파티", "로컬"]);
        for (c, ch) in [
            (SendChatChannel::World, Channel::World),
            (SendChatChannel::Guild, Channel::Guild),
            (SendChatChannel::Party, Channel::Party),
            (SendChatChannel::Say, Channel::Local),
        ] {
            assert_eq!(c.label(), ch.label());
        }
    }

    #[test]
    fn from_channel_maps_every_channel_but_beginner() {
        assert_eq!(
            SendChatChannel::from_channel(Channel::World),
            Some(SendChatChannel::World)
        );
        assert_eq!(
            SendChatChannel::from_channel(Channel::Guild),
            Some(SendChatChannel::Guild)
        );
        assert_eq!(
            SendChatChannel::from_channel(Channel::Party),
            Some(SendChatChannel::Party)
        );
        assert_eq!(
            SendChatChannel::from_channel(Channel::Local),
            Some(SendChatChannel::Say)
        );
        assert_eq!(SendChatChannel::from_channel(Channel::Beginner), None);
        let none: Vec<_> = Channel::ALL
            .into_iter()
            .filter(|&c| SendChatChannel::from_channel(c).is_none())
            .collect();
        assert_eq!(none, [Channel::Beginner]);
    }

    #[test]
    fn max_is_140() {
        assert_eq!(MAX_TEXT_UTF16, 140);
    }

    #[test]
    fn prepare_trims_and_replaces_line_breaks() {
        assert_eq!(prepare_text("  hi  "), Ok("hi".to_string()));
        assert_eq!(prepare_text("a\r\nb"), Ok("a  b".to_string()));
        assert_eq!(prepare_text("a\nb"), Ok("a b".to_string()));
        assert_eq!(prepare_text("\u{3000}hi\u{3000}"), Ok("hi".to_string()));
    }

    #[test]
    fn prepare_replaces_each_control_character_with_one_space() {
        for c in [
            '\t', '\u{0}', '\u{2028}', '\u{2029}', '\u{1}', '\u{1f}', '\u{7f}', '\u{85}', '\u{9f}',
            '\u{b}', '\u{c}',
        ] {
            assert_eq!(
                prepare_text(&format!("a{c}b")),
                Ok("a b".to_string()),
                "{c:?}"
            );
        }
        assert_eq!(prepare_text("a\t\u{0}b"), Ok("a  b".to_string()));
        assert_eq!(prepare_text("a\r\n\tb"), Ok("a   b".to_string()));
    }

    #[test]
    fn prepare_replaces_before_trimming() {
        assert_eq!(prepare_text("\u{0}hi\u{1f}"), Ok("hi".to_string()));
        for t in ["\u{0}", "\t\u{0}\u{2028}", "\u{7f}\n"] {
            assert_eq!(prepare_text(t), Err(SendTextError::Empty), "{t:?}");
        }
    }

    #[test]
    fn prepare_counts_replaced_control_characters() {
        let a = |n: usize| "a".repeat(n);
        assert_eq!(
            prepare_text(&format!("{}\t{}", a(70), a(70))),
            Err(SendTextError::TooLong { len: 141 })
        );
        assert_eq!(
            prepare_text(&(a(139) + "\u{0}a")),
            Err(SendTextError::TooLong { len: 141 })
        );
        assert!(prepare_text(&(a(140) + "\u{0}")).is_ok());
        assert!(prepare_text(&(a(70) + "\u{2028}" + &a(69))).is_ok());
    }

    #[test]
    fn prepare_leaves_no_control_character() {
        for c in ('\u{0}'..='\u{9f}')
            .filter(|c| c.is_control())
            .chain(['\u{2028}', '\u{2029}'])
        {
            let p = prepare_text(&format!("a{c}b")).unwrap();
            assert_eq!(p, "a b", "{c:?}");
            assert!(!p
                .chars()
                .any(|x| x.is_control() || x == '\u{2028}' || x == '\u{2029}'));
        }
    }

    #[test]
    fn prepare_keeps_format_characters() {
        assert_eq!(prepare_text("a\u{200b}b"), Ok("a\u{200b}b".to_string()));
    }

    #[test]
    fn prepare_refuses_empty() {
        for t in ["", "   ", "\n\r  "] {
            assert_eq!(prepare_text(t), Err(SendTextError::Empty));
        }
    }

    #[test]
    fn prepare_counts_utf16_units_after_preparation() {
        assert!(prepare_text(&"a".repeat(140)).is_ok());
        assert_eq!(
            prepare_text(&"a".repeat(141)),
            Err(SendTextError::TooLong { len: 141 })
        );
        assert!(prepare_text(&"あ".repeat(140)).is_ok());
        assert!(prepare_text(&"😀".repeat(70)).is_ok());
        assert_eq!(
            prepare_text(&"😀".repeat(71)),
            Err(SendTextError::TooLong { len: 142 })
        );
        assert!(prepare_text(&format!("{}\n", "a".repeat(140))).is_ok());
        assert_eq!(
            prepare_text(&format!("{}\n{}", "a".repeat(70), "a".repeat(70))),
            Err(SendTextError::TooLong { len: 141 })
        );
    }

    #[test]
    fn errors_display_one_korean_sentence() {
        assert_eq!(SendTextError::Empty.to_string(), "보낼 내용이 없습니다.");
        assert_eq!(
            SendTextError::TooLong { len: 141 }.to_string(),
            "메시지가 너무 깁니다. 140자 이하로 줄여 주세요 (현재 141자)."
        );
        assert_eq!(
            SendTextError::TooSoon { wait_ms: 1200 }.to_string(),
            "너무 빨리 보내고 있습니다. 1.2초 뒤에 다시 시도하세요."
        );
        assert!(SendTextError::TooSoon { wait_ms: 1 }
            .to_string()
            .contains("0.1초"));
        assert!(SendTextError::TooSoon { wait_ms: 1500 }
            .to_string()
            .contains("1.5초"));
        for e in [
            SendTextError::Empty,
            SendTextError::TooLong { len: 141 },
            SendTextError::TooSoon { wait_ms: 1500 },
        ] {
            assert!(!e.to_string().contains('\n'));
            let _: &dyn std::error::Error = &e;
        }
    }
}
