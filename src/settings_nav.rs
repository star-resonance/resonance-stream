//! The settings view's categories: the sidebar on the left, one pane on the
//! right. Pure, so it is tested on the host.

/// A settings category: one sidebar entry, one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsCategory {
    #[default]
    Translation,
    Chat,
    Keywords,
    Appearance,
    Blocked,
    Data,
}

impl SettingsCategory {
    /// Every category, in sidebar order.
    pub const ALL: [SettingsCategory; 6] = [
        SettingsCategory::Translation,
        SettingsCategory::Chat,
        SettingsCategory::Keywords,
        SettingsCategory::Appearance,
        SettingsCategory::Blocked,
        SettingsCategory::Data,
    ];

    /// The sidebar label and the pane's heading.
    pub fn title(self) -> &'static str {
        match self {
            SettingsCategory::Translation => "번역",
            SettingsCategory::Chat => "채팅",
            SettingsCategory::Keywords => "키워드",
            SettingsCategory::Appearance => "화면",
            SettingsCategory::Blocked => "차단 목록",
            SettingsCategory::Data => "데이터 및 개발자",
        }
    }

    /// The line under the pane's heading.
    pub fn description(self) -> &'static str {
        match self {
            SettingsCategory::Translation => {
                "AI 번역 사용 여부, 연산 장치와 VRAM 사용량을 설정합니다."
            }
            SettingsCategory::Chat => {
                "글꼴 크기, 메시지 간격 등 채팅 목록이 보이는 방식을 설정합니다."
            }
            SettingsCategory::Keywords => "알림을 받을 키워드와 강조할 단어를 관리합니다.",
            SettingsCategory::Appearance => "오버레이 창의 동작, 단축키, 테마를 설정합니다.",
            SettingsCategory::Blocked => "차단한 사용자와 그 메시지를 표시하는 방식을 관리합니다.",
            SettingsCategory::Data => {
                "사용자 사전, 채팅 기록, 네트워크 어댑터와 디버그 옵션입니다."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidebar_lists_every_category_once_starting_with_translation() {
        use SettingsCategory::*;
        assert_eq!(
            SettingsCategory::ALL,
            [
                Translation,
                Chat,
                Keywords,
                Appearance,
                Blocked,
                GuildRelay,
                Data
            ]
        );
        assert_eq!(SettingsCategory::default(), Translation);
    }

    #[test]
    fn every_category_has_its_own_title_and_a_description() {
        let titles: std::collections::HashSet<_> =
            SettingsCategory::ALL.iter().map(|c| c.title()).collect();
        assert_eq!(titles.len(), SettingsCategory::ALL.len());
        assert!(SettingsCategory::ALL
            .iter()
            .all(|c| !c.title().is_empty() && !c.description().is_empty()));
    }
}
