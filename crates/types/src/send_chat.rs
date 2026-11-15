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
}
