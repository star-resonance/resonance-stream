//! The cheat sheet: class and dungeon names in Korean (the main name) with their
//! Japanese names -- official and fan names -- so a Korean player can read (and
//! type) what Japanese players say. Static data plus the search over it -- pure,
//! host-tested.
//!
//! The lists are NOT yet the game's official names: they were assembled from
//! fan sites while the official ones were out of reach, and hold only pairs
//! whose two names plainly name the same thing. Check against the official
//! sites, then add the rest here (and clear [`SOURCE_NOTE`]).

/// Shown under the lists until the data is checked against the official sites.
pub const SOURCE_NOTE: &str = "공식 사이트 확인 전 · 팬 사이트 기준 일부 목록";

/// One thing, named in both languages: the Korean name is the main one, and
/// `ja` lists its Japanese names -- the official one first, then the fan names
/// the players write in chat. `children` are what it holds: a class's two
/// specializations ("trees"), a dungeon's stages; a child has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub ko: &'static str,
    pub ja: &'static [&'static str],
    pub children: &'static [Entry],
}

impl Entry {
    /// The official Japanese name.
    pub fn official(&self) -> &'static str {
        self.ja[0]
    }

    /// The other Japanese names: what the players call it.
    pub fn fan_names(&self) -> &'static [&'static str] {
        &self.ja[1..]
    }
}

const fn entry(ko: &'static str, ja: &'static [&'static str]) -> Entry {
    Entry {
        ko,
        ja,
        children: &[],
    }
}

/// An entry with children: a class and its trees, a dungeon and its stages.
const fn with_children(
    ko: &'static str,
    ja: &'static [&'static str],
    children: &'static [Entry],
) -> Entry {
    Entry { ko, ja, children }
}

pub const CLASSES: &[Entry] = &[
    with_children(
        "트윈 스트라이커",
        &["ツインストライカー"],
        &[entry("무상", &["双炎"]), entry("적홍", &["炎舞"])],
    ),
    with_children(
        "스톰 블레이드",
        &["ストームブレイド"],
        &[entry("발도", &["雷刃"]), entry("월광", &["月影"])],
    ),
    with_children(
        "윈드 나이트",
        &["ゲイルランサー"],
        &[entry("질풍", &["烈風"]), entry("난무", &["乱風"])],
    ),
    with_children(
        "프로스트 메이지",
        &["フロストメイジ"],
        &[entry("얼음창", &["氷牙"]), entry("얼음빔", &["霜天"])],
    ),
    with_children(
        "디바인 아처",
        &["ディバインアーチャー"],
        &[entry("늑대활", &["狼弓"]), entry("매활", &["鷹弓"])],
    ),
    with_children(
        "헤비 가디언",
        &["ヘヴィガーディアン"],
        &[entry("방패", &["剛身"]), entry("가드", &["剛守"])],
    ),
    with_children(
        "실드 나이트",
        &["シールドファイター"],
        &[entry("방패", &["光砕"]), entry("광휘", &["光盾"])],
    ),
    with_children(
        "실반 오라클",
        &["ヴァーダントオラクル"],
        &[entry("심판", &["威咲", "イサキ"]), entry("치유", &["森癒"])],
    ),
    with_children(
        "비트 퍼포머",
        &["ビートパフォーマー"],
        &[entry("음파", &["狂音"]), entry("협주", &["響奏"])],
    ),
];

/// What a group of the list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// A list that is not split (the classes): no header.
    Plain,
    /// Always available (상시): always open.
    Always,
    /// A season's list. The latest is open; the ones that have elapsed fold.
    Season(u8),
}

/// A titled run of entries: one season's dungeons, say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub title: &'static str,
    pub kind: GroupKind,
    pub entries: &'static [Entry],
}

/// The dungeons: always-available first, then the seasons, newest first. A new
/// season goes right after 상시.
pub const DUNGEON_GROUPS: &[Group] = &[
    Group {
        title: "상시",
        kind: GroupKind::Always,
        entries: &[
            entry("개척", &["開拓"]),
            entry("불안정", &["不安定"]),
            entry("월드 보스", &["ワールドレイド"]),
        ],
    },
    Group {
        title: "시즌3",
        kind: GroupKind::Season(3),
        entries: &[
            with_children(
                "극한공간",
                &["極限空間"],
                &[
                    entry("저주받은 무덤", &["呪われし煌墓", "墓"]),
                    entry("기계화 처리소", &["機械化工場", "工場"]),
                    entry("침식 티나", &["蝕・ティナの精神領域", "ティナ"]),
                    entry("안개 속 사냥터", &["霧海の猟場", "霧海"]),
                    entry("침식 거탑", &["蝕・巨塔の遺跡", "巨塔"]),
                    entry("환해 암초(나뽀)", &["珊瑚岩の谷", "珊瑚"]),
                    entry("우리보", &["ウリボ"]),
                ],
            ),
            with_children(
                "환상꿈 레이드",
                &["幻夢レイド"],
                &[
                    entry("시작", &["始"]),
                    entry("계속", &["継"]),
                    entry("종결", &["終"]),
                ],
            ),
            entry("미망의 숲", &["迷妄の森", "迷妄"]),
        ],
    },
];

/// The classes, as the one group of their section.
const CLASS_GROUPS: &[Group] = &[Group {
    title: "",
    kind: GroupKind::Plain,
    entries: CLASSES,
}];

/// The line pieces drawn in front of a child row, so a class and its trees (a
/// dungeon and its stages) read as a tree. All are absolutely placed in the
/// row, on one vertical line: the elbow runs from the row's top down to its
/// middle and across to the name; the stem carries the line on down to the
/// next child. The last child has no stem, so the line ends at it.
const TREE_ELBOW: &str =
    "absolute left-4 top-0 h-1/2 w-3 border-l border-b border-base-content/25 rounded-bl-md";
const TREE_STEM: &str = "absolute left-4 top-1/2 bottom-0 border-l border-base-content/25";

/// The pieces for a child row; `last` is the last child of its parent.
pub fn tree_lines(last: bool) -> &'static [&'static str] {
    if last {
        &[TREE_ELBOW]
    } else {
        &[TREE_ELBOW, TREE_STEM]
    }
}

/// The newest season among `groups`.
pub fn latest_season(groups: &[Group]) -> Option<u8> {
    groups
        .iter()
        .filter_map(|g| match g.kind {
            GroupKind::Season(n) => Some(n),
            _ => None,
        })
        .max()
}

/// Whether a group can be folded: only a season that is not the latest.
pub fn collapsible(kind: GroupKind, latest: Option<u8>) -> bool {
    matches!(kind, GroupKind::Season(n) if Some(n) != latest)
}

/// Whether a group shows its entries: a group that cannot fold always does; a
/// folded season opens when the user opened it or a search is showing its matches.
pub fn is_open(collapsible: bool, opened_by_user: bool, searching: bool) -> bool {
    !collapsible || opened_by_user || searching
}

/// A list on the cheat sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Classes,
    Dungeons,
}

impl Section {
    pub const ALL: [Section; 2] = [Section::Classes, Section::Dungeons];

    pub fn label(self) -> &'static str {
        match self {
            Section::Classes => "직업",
            Section::Dungeons => "던전",
        }
    }

    pub fn groups(self) -> &'static [Group] {
        match self {
            Section::Classes => CLASS_GROUPS,
            Section::Dungeons => DUNGEON_GROUPS,
        }
    }
}

/// What a search compares: lower case, without spaces or the dots that
/// separate parts of a name ("스톰 블레이드" is found by "스톰블레이드").
fn key(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '・' | '·' | '･' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// One search result: an entry and the children to show under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub entry: Entry,
    pub children: Vec<Entry>,
}

/// The hits inside one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupHits {
    pub group: Group,
    pub hits: Vec<Hit>,
    /// Whether this group can fold (an elapsed season), see [`collapsible`].
    pub collapsible: bool,
}

/// Whether `query` (already a [`key`]) is in the entry's own names: Korean,
/// official Japanese or a fan name.
fn names_match(e: &Entry, query: &str) -> bool {
    key(e.ko).contains(query) || e.ja.iter().any(|n| key(n).contains(query))
}

/// The entries of `section` that `query` finds, in either language, by group
/// (a group without a hit is left out); everything for an empty query. An entry
/// found by its own name shows all its children; one found only through a child
/// shows just the children that matched.
pub fn search(section: Section, query: &str) -> Vec<GroupHits> {
    let query = key(query);
    let latest = latest_season(section.groups());
    section
        .groups()
        .iter()
        .filter_map(|&group| {
            let hits: Vec<Hit> = group
                .entries
                .iter()
                .filter_map(|&entry| {
                    if query.is_empty() || names_match(&entry, &query) {
                        return Some(Hit {
                            entry,
                            children: entry.children.to_vec(),
                        });
                    }
                    let children: Vec<Entry> = entry
                        .children
                        .iter()
                        .copied()
                        .filter(|c| names_match(c, &query))
                        .collect();
                    (!children.is_empty()).then_some(Hit { entry, children })
                })
                .collect();
            (!hits.is_empty()).then(|| GroupHits {
                group,
                hits,
                collapsible: collapsible(group.kind, latest),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_types::contains_japanese;

    /// Every entry of every group of every section.
    fn all() -> impl Iterator<Item = Entry> {
        Section::ALL
            .into_iter()
            .flat_map(|s| s.groups().iter())
            .flat_map(|g| g.entries.iter().copied())
    }

    fn has_korean(text: &str) -> bool {
        text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
    }

    /// A search's hits, all groups together.
    fn found(section: Section, query: &str) -> Vec<Hit> {
        search(section, query)
            .into_iter()
            .flat_map(|g| g.hits)
            .collect()
    }

    /// The Korean names of a search's hits, entry level only.
    fn names(section: Section, query: &str) -> Vec<&'static str> {
        found(section, query).iter().map(|h| h.entry.ko).collect()
    }

    fn group(title: &str) -> Group {
        *Section::Dungeons
            .groups()
            .iter()
            .find(|g| g.title == title)
            .unwrap_or_else(|| panic!("no group {title}"))
    }

    fn entry_of(group: &Group, ko: &str) -> Entry {
        *group.entries.iter().find(|e| e.ko == ko).unwrap()
    }

    /// `ko` and `ja` of each child, as given.
    fn kids(e: Entry) -> Vec<(&'static str, Vec<&'static str>)> {
        e.children.iter().map(|c| (c.ko, c.ja.to_vec())).collect()
    }

    #[test]
    fn every_entry_and_child_has_a_korean_name_and_japanese_names() {
        for e in all() {
            for x in std::iter::once(e).chain(e.children.iter().copied()) {
                assert!(has_korean(x.ko), "{x:?}");
                assert!(!x.ja.is_empty(), "{x:?}");
                assert!(x.ja.iter().all(|n| contains_japanese(n)), "{x:?}");
            }
            for c in e.children {
                assert!(c.children.is_empty(), "children do not nest: {c:?}");
            }
        }
    }

    #[test]
    fn the_first_japanese_name_is_the_official_one_and_the_rest_are_fan_names() {
        let oracle = CLASSES.iter().find(|c| c.ko == "실반 오라클").unwrap();
        let tree = oracle.children[0];
        assert_eq!(tree.ko, "심판");
        assert_eq!(tree.official(), "威咲");
        assert_eq!(tree.fan_names(), ["イサキ"]);
        assert_eq!(tree.ja, ["威咲", "イサキ"]);
        assert!(oracle.children[1].fan_names().is_empty());
        assert_eq!(CLASSES[0].official(), "ツインストライカー");
    }

    #[test]
    fn the_nine_classes_each_have_two_trees() {
        assert_eq!(CLASSES.len(), 9);
        for class in CLASSES {
            assert_eq!(class.children.len(), 2, "{class:?}");
        }
    }

    #[test]
    fn the_classes_and_their_trees_are_listed_as_given() {
        let class = |ja: &str| {
            CLASSES
                .iter()
                .find(|c| c.official() == ja)
                .copied()
                .unwrap()
        };
        let trees = |ja: &str| {
            class(ja)
                .children
                .iter()
                .map(|t| (t.official(), t.ko))
                .collect::<Vec<_>>()
        };
        assert_eq!(class("ゲイルランサー").ko, "윈드 나이트");
        assert_eq!(class("シールドファイター").ko, "실드 나이트");
        assert_eq!(
            trees("ツインストライカー"),
            [("双炎", "무상"), ("炎舞", "적홍")]
        );
        assert_eq!(
            trees("ストームブレイド"),
            [("雷刃", "발도"), ("月影", "월광")]
        );
        assert_eq!(
            trees("ゲイルランサー"),
            [("烈風", "질풍"), ("乱風", "난무")]
        );
        assert_eq!(
            trees("フロストメイジ"),
            [("氷牙", "얼음창"), ("霜天", "얼음빔")]
        );
        assert_eq!(
            trees("ディバインアーチャー"),
            [("狼弓", "늑대활"), ("鷹弓", "매활")]
        );
        assert_eq!(
            trees("ヘヴィガーディアン"),
            [("剛身", "방패"), ("剛守", "가드")]
        );
        assert_eq!(
            trees("シールドファイター"),
            [("光砕", "방패"), ("光盾", "광휘")]
        );
        assert_eq!(
            trees("ヴァーダントオラクル"),
            [("威咲", "심판"), ("森癒", "치유")]
        );
        assert_eq!(
            trees("ビートパフォーマー"),
            [("狂音", "음파"), ("響奏", "협주")]
        );
    }

    #[test]
    fn the_dungeons_are_always_available_then_the_seasons_newest_first() {
        let groups = Section::Dungeons.groups();
        assert_eq!(
            groups.iter().map(|g| g.title).collect::<Vec<_>>(),
            ["상시", "시즌3"]
        );
        assert_eq!(groups[0].kind, GroupKind::Always);
        assert_eq!(groups[1].kind, GroupKind::Season(3));
        // The classes are one list without a header.
        let classes = Section::Classes.groups();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].kind, GroupKind::Plain);
        assert_eq!(classes[0].title, "");
        // Seasons run newest first, so the list reads top down.
        let seasons: Vec<u8> = Section::Dungeons
            .groups()
            .iter()
            .filter_map(|g| match g.kind {
                GroupKind::Season(n) => Some(n),
                _ => None,
            })
            .collect();
        assert!(seasons.windows(2).all(|w| w[0] > w[1]), "{seasons:?}");
    }

    #[test]
    fn the_always_available_dungeons_are_listed_as_given() {
        let g = group("상시");
        let list: Vec<_> = g.entries.iter().map(|e| (e.ko, e.ja.to_vec())).collect();
        assert_eq!(
            list,
            [
                ("개척", vec!["開拓"]),
                ("불안정", vec!["不安定"]),
                ("월드 보스", vec!["ワールドレイド"]),
            ]
        );
        assert!(g.entries.iter().all(|e| e.children.is_empty()));
    }

    #[test]
    fn season_three_is_listed_as_given() {
        let g = group("시즌3");
        let titles: Vec<_> = g.entries.iter().map(|e| e.ko).collect();
        assert_eq!(titles, ["극한공간", "환상꿈 레이드", "미망의 숲"]);
        let extreme = entry_of(&g, "극한공간");
        assert_eq!(extreme.ja, ["極限空間"]);
        assert_eq!(
            kids(extreme),
            [
                ("저주받은 무덤", vec!["呪われし煌墓", "墓"]),
                ("기계화 처리소", vec!["機械化工場", "工場"]),
                ("침식 티나", vec!["蝕・ティナの精神領域", "ティナ"]),
                ("안개 속 사냥터", vec!["霧海の猟場", "霧海"]),
                ("침식 거탑", vec!["蝕・巨塔の遺跡", "巨塔"]),
                ("환해 암초(나뽀)", vec!["珊瑚岩の谷", "珊瑚"]),
                ("우리보", vec!["ウリボ"]),
            ]
        );
        let raid = entry_of(&g, "환상꿈 레이드");
        assert_eq!(raid.ja, ["幻夢レイド"]);
        assert_eq!(
            kids(raid),
            [
                ("시작", vec!["始"]),
                ("계속", vec!["継"]),
                ("종결", vec!["終"]),
            ]
        );
        let forest = entry_of(&g, "미망의 숲");
        assert_eq!(forest.ja, ["迷妄の森", "迷妄"]);
        assert!(forest.children.is_empty());
    }

    #[test]
    fn the_dungeons_of_older_seasons_are_gone() {
        for old in ["고블린", "ゴブリン", "거룡", "巨竜", "거탑 유적"] {
            assert!(found(Section::Dungeons, old).is_empty(), "{old}");
        }
    }

    #[test]
    fn the_latest_season_is_the_highest_number_and_only_older_seasons_collapse() {
        let groups = Section::Dungeons.groups();
        assert_eq!(latest_season(groups), Some(3));
        assert_eq!(latest_season(Section::Classes.groups()), None);
        // Always-available lists and the latest season stay open ...
        assert!(!collapsible(GroupKind::Always, Some(3)));
        assert!(!collapsible(GroupKind::Plain, None));
        assert!(!collapsible(GroupKind::Season(3), Some(3)));
        // ... older seasons are the ones that fold.
        assert!(collapsible(GroupKind::Season(2), Some(3)));
        assert!(collapsible(GroupKind::Season(1), Some(3)));
    }

    #[test]
    fn a_folded_season_opens_by_a_click_or_a_search_and_nothing_else_closes() {
        // (collapsible, opened by the user, searching) -> open?
        assert!(!is_open(true, false, false));
        assert!(is_open(true, true, false), "the user opened it");
        assert!(is_open(true, false, true), "a search shows its matches");
        assert!(is_open(false, false, false), "never folded");
        assert!(is_open(false, false, true));
    }

    #[test]
    fn a_search_groups_its_hits_and_marks_the_groups_that_can_fold() {
        let groups = search(Section::Dungeons, "");
        assert_eq!(groups.len(), 2);
        assert!(
            groups.iter().all(|g| !g.collapsible),
            "only season 3 exists: the latest"
        );
        assert_eq!(groups[0].group.title, "상시");
        assert_eq!(groups[1].group.title, "시즌3");
        let g = &groups[1];
        assert_eq!(g.hits.len(), 3);
        assert_eq!(g.hits[0].children.len(), 7);
    }

    #[test]
    fn a_search_leaves_out_the_groups_without_a_match() {
        let groups = search(Section::Dungeons, "개척");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group.title, "상시");
        assert!(search(Section::Dungeons, "zzz").is_empty());
    }

    #[test]
    fn a_child_found_by_a_fan_name_shows_its_parent_with_only_that_child() {
        let hits = found(Section::Dungeons, "墓");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.ko, "극한공간");
        let kids: Vec<_> = hits[0].children.iter().map(|c| c.ko).collect();
        assert_eq!(kids, ["저주받은 무덤"]);
    }

    #[test]
    fn a_word_in_several_children_finds_them_all_under_one_parent() {
        let hits = found(Section::Dungeons, "침식");
        assert_eq!(hits.len(), 1);
        let kids: Vec<_> = hits[0].children.iter().map(|c| c.ko).collect();
        assert_eq!(kids, ["침식 티나", "침식 거탑"]);
        // The dot in 蝕・巨塔の遺跡 does not get in the way.
        assert_eq!(names(Section::Dungeons, "蝕巨塔"), ["극한공간"]);
    }

    #[test]
    fn a_parent_found_by_its_own_name_keeps_all_its_children() {
        let hits = found(Section::Dungeons, "환상꿈");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].children.len(), 3);
    }

    #[test]
    fn no_name_is_listed_twice_in_a_group_in_a_class_list_or_in_an_entry() {
        let distinct = |a: &Entry, b: &Entry| {
            key(a.ko) != key(b.ko) && a.ja.iter().all(|x| !b.ja.iter().any(|y| key(x) == key(y)))
        };
        for section in Section::ALL {
            for g in section.groups() {
                let entries = g.entries;
                for (i, a) in entries.iter().enumerate() {
                    for b in &entries[i + 1..] {
                        assert!(distinct(a, b), "{a:?} {b:?}");
                    }
                    // "방패" is a tree of two classes -- but never twice in one.
                    for (j, x) in a.children.iter().enumerate() {
                        for y in &a.children[j + 1..] {
                            assert!(distinct(x, y), "{x:?} {y:?}");
                        }
                    }
                    // One entry never lists the same Japanese name twice.
                    for x in std::iter::once(a).chain(a.children) {
                        for (k, n) in x.ja.iter().enumerate() {
                            assert!(!x.ja[k + 1..].iter().any(|m| key(m) == key(n)), "{x:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn an_empty_search_lists_everything_with_all_children() {
        for section in Section::ALL {
            for q in ["", "  "] {
                let total: usize = search(section, q).iter().map(|g| g.hits.len()).sum();
                let want: usize = section.groups().iter().map(|g| g.entries.len()).sum();
                assert_eq!(total, want);
                for hit in found(section, q) {
                    assert_eq!(hit.children.len(), hit.entry.children.len());
                }
            }
        }
    }

    #[test]
    fn a_search_matches_either_language() {
        let by_ja = found(Section::Classes, "ストーム");
        let by_ko = found(Section::Classes, "스톰");
        assert_eq!(by_ja, by_ko);
        assert_eq!(by_ja.len(), 1);
        assert!(by_ja[0].entry.ko.contains("스톰"));
        assert_eq!(by_ja[0].children.len(), 2, "its own name keeps both trees");
    }

    #[test]
    fn a_hit_on_a_tree_name_shows_its_class_with_only_that_tree() {
        for query in ["雷刃", "발도"] {
            assert_eq!(names(Section::Classes, query), ["스톰 블레이드"], "{query}");
            let hits = found(Section::Classes, query);
            let trees: Vec<_> = hits[0].children.iter().map(|t| t.ko).collect();
            assert_eq!(trees, ["발도"], "{query}");
        }
    }

    #[test]
    fn a_tree_name_shared_by_two_classes_finds_both() {
        assert_eq!(
            names(Section::Classes, "방패"),
            ["헤비 가디언", "실드 나이트"]
        );
        for hit in found(Section::Classes, "방패") {
            assert_eq!(hit.children.len(), 1);
            assert_eq!(hit.children[0].ko, "방패");
        }
    }

    #[test]
    fn a_tree_is_found_by_its_korean_name_its_official_name_or_a_fan_name() {
        for query in ["심판", "威咲", "イサキ"] {
            assert_eq!(names(Section::Classes, query), ["실반 오라클"], "{query}");
            let hits = found(Section::Classes, query);
            assert_eq!(hits[0].children.len(), 1, "{query}");
            assert_eq!(hits[0].children[0].ja, ["威咲", "イサキ"]);
        }
    }

    #[test]
    fn a_search_ignores_spaces_dots_and_case() {
        assert_eq!(names(Section::Classes, "스톰블레이드").len(), 1);
        assert_eq!(
            names(Section::Dungeons, "침식티나"),
            ["극한공간"],
            "a space"
        );
        assert_eq!(names(Section::Dungeons, "蝕ティナ"), ["극한공간"], "a dot");
    }

    #[test]
    fn a_search_only_looks_in_its_own_section() {
        assert!(found(Section::Classes, "극한").is_empty());
        assert_eq!(found(Section::Dungeons, "극한").len(), 1);
    }

    #[test]
    fn the_sections_are_named() {
        assert_eq!(Section::ALL.map(|s| s.label()), ["직업", "던전"]);
    }

    #[test]
    fn a_child_hangs_from_a_line_and_only_the_last_one_ends_it() {
        let classes = |c: &str| c.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        let has = |c: &[String], need: &str| c.contains(&need.to_string());
        let middle = tree_lines(false);
        let last = tree_lines(true);
        // Every child gets the elbow: a line down to its row's middle, then across.
        assert_eq!(middle[0], last[0]);
        let elbow = classes(last[0]);
        for need in ["absolute", "top-0", "h-1/2", "border-l", "border-b"] {
            assert!(has(&elbow, need), "{need}");
        }
        // The line goes on past a middle child, and stops at the last one's elbow.
        assert_eq!(last.len(), 1);
        assert_eq!(middle.len(), 2);
        let stem = classes(middle[1]);
        for need in ["absolute", "top-1/2", "bottom-0", "border-l"] {
            assert!(has(&stem, need), "{need}");
        }
        // The elbow and the stem sit on the same vertical line.
        let left = |c: &[String]| c.iter().find(|x| x.starts_with("left-")).cloned();
        assert!(left(&elbow).is_some());
        assert_eq!(left(&elbow), left(&stem));
    }
    // --- the cheat sheet and the dictionary name the same things (roadmap N-3) ----------------------
    // Two hand-maintained copies of one set of names. They drifted once (ten class trees, one spelling);
    // this keeps them from drifting silently again: every Japanese name here is in the dictionary with
    // the same Korean, or is listed below with the reason it is not.

    const DICTIONARY: &str = include_str!("../metadata/custom_dict.json");

    /// Korean as compared: a label's "(nickname)" and the spaces do not count.
    fn plain(korean: &str) -> String {
        let mut out = String::new();
        let mut depth = 0;
        for c in korean.chars() {
            match c {
                '(' => depth += 1,
                ')' if depth > 0 => depth -= 1,
                c if depth == 0 && !c.is_whitespace() => out.push(c),
                _ => {}
            }
        }
        out
    }

    /// Every Japanese key of the dictionary, with each Korean it has, across all categories.
    fn dictionary() -> std::collections::BTreeMap<String, Vec<String>> {
        let root: serde_json::Value =
            serde_json::from_str(DICTIONARY).expect("the dictionary parses");
        let mut all = std::collections::BTreeMap::<String, Vec<String>>::new();
        for terms in root.as_object().expect("categories").values() {
            for (ja, ko) in terms.as_object().expect("terms") {
                all.entry(ja.clone())
                    .or_default()
                    .push(ko.as_str().expect("text").to_string());
            }
        }
        all
    }

    /// Cheat-sheet names the dictionary does not match, each with why. A class tree is also looked up with
    /// the `型` the dictionary writes after it (`雷刃` / `雷刃型`). Empty a line when it is settled: the test
    /// fails while a name is listed here and no longer needs to be.
    const KNOWN_DISAGREEMENTS: &[(&str, &str)] = &[
        // Short forms that would mean something else, or hit ordinary words, as dictionary terms.
        (
            "ティナ",
            "already the NPC (티나); the cheat sheet uses it for the 침식 티나 dungeon",
        ),
        (
            "巨塔",
            "the dictionary's 거탑 is the tower; the cheat sheet's 침식 거탑 is one dungeon of it",
        ),
        (
            "墓",
            "one character: it would be replaced inside ordinary words",
        ),
        (
            "始",
            "one character: it would be replaced inside ordinary words (始める)",
        ),
        (
            "継",
            "one character: it would be replaced inside ordinary words",
        ),
        (
            "終",
            "one character: it would be replaced inside ordinary words (終わり)",
        ),
        // Not a season 3 name; nobody decided to add it.
        (
            "開拓",
            "always-available dungeon, two characters of an ordinary word: not added",
        ),
    ];

    /// Japanese names of the cheat sheet that the dictionary does not carry with the same Korean.
    fn disagreements() -> std::collections::BTreeSet<String> {
        let words = dictionary();
        let mut out = std::collections::BTreeSet::new();
        let entries = all().flat_map(|e| std::iter::once(e).chain(e.children.iter().copied()));
        for e in entries {
            for ja in e.ja {
                let matches = |key: &str| {
                    words
                        .get(key)
                        .is_some_and(|korean| korean.iter().any(|k| plain(k) == plain(e.ko)))
                };
                if !(matches(ja) || matches(&format!("{ja}型"))) {
                    out.insert((*ja).to_string());
                }
            }
        }
        out
    }

    #[test]
    fn the_cheat_sheet_and_the_dictionary_name_things_alike_except_where_listed() {
        let known: std::collections::BTreeSet<String> = KNOWN_DISAGREEMENTS
            .iter()
            .map(|(ja, _)| (*ja).to_string())
            .collect();
        let found = disagreements();
        let new: Vec<_> = found.difference(&known).collect();
        let settled: Vec<_> = known.difference(&found).collect();
        assert!(
            new.is_empty(),
            "in the cheat sheet, not (or differently) in the dictionary: {new:?}"
        );
        assert!(
            settled.is_empty(),
            "listed as disagreeing but they agree now; remove them: {settled:?}"
        );
    }

    #[test]
    fn every_listed_disagreement_says_why() {
        for (ja, why) in KNOWN_DISAGREEMENTS {
            assert!(why.chars().count() >= 12, "{ja}: give the reason");
        }
    }
}
