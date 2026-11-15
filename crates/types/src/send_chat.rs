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
