//! The custom dictionary this repo publishes (`metadata/custom_dict.json`): what it holds, and the two
//! rules that keep it safe to ship to every user.
//!
//! The app flattens every category into one map keyed by the Japanese text and shields each term it
//! finds in a chat line, so a key must mean one thing, and a one-character key hits ordinary words
//! (`始` in `始める`, `終` in `終わり`).

use resonance_core::text::Dictionary;
use std::collections::{BTreeMap, BTreeSet};

const SOURCE: &str = include_str!("../../../metadata/custom_dict.json");

/// `{ category: { ja: ko } }`, as the file has it.
fn categories() -> BTreeMap<String, BTreeMap<String, String>> {
    serde_json::from_str(SOURCE).expect("the dictionary is categorised JSON")
}

/// Every Japanese key with each Korean it has, across all categories.
fn meanings() -> BTreeMap<String, BTreeSet<String>> {
    let mut all: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for terms in categories().values() {
        for (ja, ko) in terms {
            all.entry(ja.clone()).or_default().insert(ko.clone());
        }
    }
    all
}

#[test]
fn the_dictionary_is_one_the_app_can_read() {
    let dictionary = Dictionary::from_json_str(SOURCE).expect("it parses");
    assert!(dictionary.len() > 100, "{} terms", dictionary.len());
}

#[test]
fn a_key_that_is_in_two_categories_has_the_same_korean_in_both() {
    // One map is built from all categories: two meanings would leave the winner to the map's order.
    let clashes: Vec<_> = meanings()
        .into_iter()
        .filter(|(_, ko)| ko.len() > 1)
        .collect();
    assert!(clashes.is_empty(), "keys with two meanings: {clashes:?}");
}

#[test]
fn the_only_one_character_terms_are_the_known_ones() {
    // A one-character term is shielded wherever the character stands, inside words too. These three
    // are deliberate (`凸` is the game's word for a breakthrough, `〆` closes a recruiting call, `＠`
    // is kept as it is); a new one needs a reason, so it is listed here on purpose.
    let one_character: BTreeSet<_> = meanings()
        .into_keys()
        .filter(|ja| ja.chars().count() == 1)
        .collect();
    let known: BTreeSet<_> = ["〆", "凸", "＠"].map(String::from).into();
    assert_eq!(one_character, known);
}

/// The two-character keys, each with why it may stay. A key like these is shielded wherever the two
/// characters stand next to each other, inside a longer word too (Japanese has no word boundary to
/// match on), so a new one is added here on purpose, with the reason it is safe.
const TWO_CHARACTER_KEYS: &[(&str, &str)] = &[
    (
        "PT",
        "the party abbreviation players write as is (파티); Latin letters, not part of Japanese words",
    ),
    (
        "募集",
        "the recruiting call (모집); a recruiting-chat word, and 모집 is also its ordinary Korean reading",
    ),
    (
        "完凸",
        "the game's word for a fully upgraded breakthrough (풀돌); not an ordinary word",
    ),
    (
        "周回",
        "repeat runs of a dungeon (주회), the game's chat term; its ordinary meaning, going around, reads the same",
    ),
    (
        "雷刃",
        "short name of the 발도 tree of the Stormblade, from the cheat sheet; only the 刃 ending is common",
    ),
    (
        "烈風",
        "short name of the 질풍 tree of the Gale Lancer, from the cheat sheet",
    ),
    (
        "乱風",
        "short name of the 난무 tree of the Gale Lancer, from the cheat sheet",
    ),
    (
        "剛守",
        "short name of the 가드 tree of the Heavy Guardian, from the cheat sheet",
    ),
    (
        "狂音",
        "short name of the 음파 tree of the Beat Performer, from the cheat sheet",
    ),
    (
        "響奏",
        "short name of the 협주 tree of the Beat Performer, from the cheat sheet",
    ),
    (
        "光砕",
        "short name of the 방패 tree of the Shield Knight, from the cheat sheet",
    ),
    (
        "光盾",
        "short name of the 광휘 tree of the Shield Knight, from the cheat sheet",
    ),
    (
        "浮島",
        "the floating islands (부유섬), a dungeon name",
    ),
    (
        "遺跡",
        "ruins (유적), a dungeon name; it reads the same in the ordinary word",
    ),
    (
        "虚飾",
        "the eroded-dungeon prefix (침식), a season 3 name",
    ),
    (
        "千夢",
        "the dream-weaving raid name (꿈엮기), a season 3 name",
    ),
    (
        "巨塔",
        "the tower (거탑); an existing spelling, kept as it was",
    ),
    (
        "巨龍",
        "the giant dragon (거룡); one of its two spellings",
    ),
    (
        "巨竜",
        "the giant dragon (거룡); the other spelling",
    ),
    (
        "暗霧",
        "the dark fog (검은 안개), a dungeon and a monster name",
    ),
    (
        "工場",
        "the factory dungeon (기계화 처리소); the ordinary word means a factory, a possible false hit",
    ),
    (
        "霧海",
        "the sea of fog hunting ground (안개 속 사냥터), a season 3 name",
    ),
    (
        "珊瑚",
        "the coral valley (환해 암초), a season 3 name; the ordinary word means coral, a possible false hit",
    ),
    (
        "迷妄",
        "the forest of delusion (미망의 숲), a season 3 name",
    ),
    (
        "鉄牙",
        "a monster name (무쇠 이빨)",
    ),
];

#[test]
fn every_two_character_term_is_listed_with_its_reason() {
    let two: BTreeSet<_> = meanings()
        .into_keys()
        .filter(|ja| ja.chars().count() == 2)
        .collect();
    let listed: BTreeSet<_> = TWO_CHARACTER_KEYS
        .iter()
        .map(|(ja, _)| (*ja).to_string())
        .collect();
    let new: Vec<_> = two.difference(&listed).collect();
    let gone: Vec<_> = listed.difference(&two).collect();
    assert!(new.is_empty(), "short keys with no reason listed: {new:?}");
    assert!(
        gone.is_empty(),
        "listed, but not in the dictionary: {gone:?}"
    );
    for (ja, why) in TWO_CHARACTER_KEYS {
        assert!(why.chars().count() >= 12, "{ja}: give the reason");
    }
}

#[test]
fn season_3_dungeons_are_named_as_the_cheat_sheet_names_them() {
    // The official Japanese name and the short name the players write, for each season 3 dungeon.
    // Left out on purpose: the one-character stage names (墓, 始, 継, 終), `ティナ` and `巨塔`, which
    // already mean the NPC and the tower (`티나`, `거탑`), and `極限空間`, spelt `극한 공간`.
    let want = [
        ("呪われし煌墓", "저주받은 무덤"),
        ("機械化工場", "기계화 처리소"),
        ("工場", "기계화 처리소"),
        ("蝕・ティナの精神領域", "침식 티나"),
        ("霧海の猟場", "안개 속 사냥터"),
        ("霧海", "안개 속 사냥터"),
        ("蝕・巨塔の遺跡", "침식 거탑"),
        ("珊瑚岩の谷", "환해 암초"),
        ("珊瑚", "환해 암초"),
        ("幻夢レイド", "환상꿈 레이드"),
        ("迷妄の森", "미망의 숲"),
        ("迷妄", "미망의 숲"),
    ];
    let all = categories();
    let dungeon = &all["dungeon"];
    for (ja, ko) in want {
        assert_eq!(dungeon.get(ja).map(String::as_str), Some(ko), "{ja}");
    }
    // What was already there is untouched.
    assert_eq!(dungeon["極限空間"], "극한 공간");
    assert_eq!(dungeon["巨塔"], "거탑");
    assert_eq!(all["npc"]["ティナ"], "티나");
}

#[test]
fn the_classes_and_their_trees_are_named_as_the_cheat_sheet_names_them() {
    let want = [
        ("ツインストライカー", "트윈 스트라이커"),
        ("双炎型", "무상"),
        ("炎舞型", "적홍"),
        ("雷刃", "발도"),
        ("烈風", "질풍"),
        ("乱風", "난무"),
        ("剛守", "가드"),
        ("狂音", "음파"),
        ("響奏", "협주"),
        // 실드 나이트: the cheat sheet is right, the dictionary had the two trees the other way round
        // (光砕型 was "광휘의 축복", 光盾型 "신성한 방패").
        ("光砕型", "방패"),
        ("光盾型", "광휘"),
        ("光砕", "방패"),
        ("光盾", "광휘"),
    ];
    let all = categories();
    let classes = &all["class"];
    for (ja, ko) in want {
        assert_eq!(classes.get(ja).map(String::as_str), Some(ko), "{ja}");
    }
}

#[test]
fn a_monster_is_named_as_what_it_is() {
    // Was "리자드맨 마법사사": a doubled syllable.
    assert_eq!(categories()["monster"]["ギルミーメイジ"], "리자드맨 마법사");
}
