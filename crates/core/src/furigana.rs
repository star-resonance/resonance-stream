//! Furigana for a Japanese chat line: the line cut into [`RubySpan`]s, each
//! piece with kanji carrying its hiragana reading.
//!
//! [`spans_from_tokens`] is the pure part: it takes a morphological analyser's
//! tokens (surface text and katakana reading) and puts the reading over the
//! kanji only, leaving okurigana (走**った**) plain. A word the analyser has no
//! reading for is read kanji by kanji in on'yomi ([`crate::kanji_on`]); so are
//! single kanji the analyser gives side by side (響奏, not ひびきそう). Which
//! analyser makes the tokens is the adapter's business, so this part is tested
//! without one.

use crate::kanji_on::on_reading;
use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use resonance_types::{contains_japanese, RubySpan};
use std::borrow::Cow;
use std::sync::OnceLock;

/// One analysed word: the text as written, and its reading in katakana when
/// the analyser knows it.
pub type Token<'a> = (&'a str, Option<&'a str>);

/// The line as [`RubySpan`]s. The spans' text, joined, is exactly the tokens'
/// surfaces, joined; neighbouring plain pieces are merged into one span.
pub fn spans_from_tokens<'a>(tokens: impl IntoIterator<Item = Token<'a>>) -> Vec<RubySpan> {
    let tokens: Vec<Token> = tokens.into_iter().collect();
    let mut out = Vec::new();
    let mut counted = false;
    for (i, &(surface, reading)) in tokens.iter().enumerate() {
        // 人, already part of the 一人 / 二人 before it.
        if std::mem::take(&mut counted) {
            continue;
        }
        if let Some(reading) = irregular_count(&tokens, i) {
            let word = format!("{surface}{}", tokens[i + 1].0);
            push(&mut out, RubySpan::with_reading(word, reading));
            counted = true;
            continue;
        }
        let reading = reading
            .filter(|r| *r != "*")
            .map(to_hiragana)
            .filter(|r| !r.is_empty() && r.chars().all(is_hiragana_or_mark));
        // A kanji the dictionary knows only apart from its neighbours (響奏:
        // 響 ひびき, 奏 そう) is no word of Japanese: on'yomi, like a coined
        // Sino-Japanese compound, where the table has one. Not a count (三人):
        // the dictionary reads the counter after a number, さんにん.
        let reading = match single_kanji(surface) {
            Some(kanji) if in_single_kanji_run(&tokens, i) && !in_count(&tokens, i) => {
                on_reading(kanji).map(str::to_string).or(reading)
            }
            _ => reading,
        };
        match reading {
            Some(reading) if surface.chars().any(is_kanji) => {
                for span in align(surface, &reading) {
                    push(&mut out, span);
                }
            }
            // The analyser does not know the word (巨塔): read each kanji on
            // its own in on'yomi, and leave what has none (kana, 畑) plain.
            None if surface.chars().any(is_kanji) => {
                for c in surface.chars() {
                    let mut buf = [0; 4];
                    let text: &str = c.encode_utf8(&mut buf);
                    match on_reading(c) {
                        Some(reading) => push(&mut out, RubySpan::with_reading(text, reading)),
                        None => push(&mut out, RubySpan::plain(text)),
                    }
                }
            }
            _ => push(&mut out, RubySpan::plain(surface)),
        }
    }
    out
}

/// The kanji when `surface` is exactly one (not 々, which repeats the kanji
/// before it).
fn single_kanji(surface: &str) -> Option<char> {
    let mut chars = surface.chars();
    let c = chars.next()?;
    (chars.next().is_none() && is_kanji(c) && c != '々').then_some(c)
}

/// Whether the token at `i` is one of two or more single-kanji tokens in a row.
fn in_single_kanji_run(tokens: &[Token], i: usize) -> bool {
    let single = |j: usize| single_kanji(tokens[j].0).is_some();
    single(i) && ((i > 0 && single(i - 1)) || (i + 1 < tokens.len() && single(i + 1)))
}

/// Whether the run of single-kanji tokens around `i` starts with a numeral.
fn in_count(tokens: &[Token], i: usize) -> bool {
    single_kanji(tokens[run_start(tokens, i)].0).is_some_and(is_numeral)
}

/// Where the run of single-kanji tokens that holds `i` starts.
fn run_start(tokens: &[Token], i: usize) -> usize {
    let mut start = i;
    while start > 0 && single_kanji(tokens[start - 1].0).is_some() {
        start -= 1;
    }
    start
}

/// The reading of the token at `i` together with the 人 after it, when the
/// two are 一人 or 二人: the dictionary has 一 and 人 apart (いち, にん), and
/// these two counts are said as words of their own. Only at the start of the
/// count: 十二人 is じゅうににん.
fn irregular_count(tokens: &[Token], i: usize) -> Option<&'static str> {
    if tokens.get(i + 1)?.0 != "人" || run_start(tokens, i) != i {
        return None;
    }
    match tokens[i].0 {
        "一" => Some("ひとり"),
        "二" => Some("ふたり"),
        _ => None,
    }
}

/// A word's surface cut so the reading sits over its kanji only. When the
/// word's kana and its reading do not line up, the whole word carries the
/// reading: a coarser ruby, never a wrong one.
fn align(surface: &str, reading: &str) -> Vec<RubySpan> {
    let runs = runs(surface);
    let reading_chars: Vec<char> = reading.chars().collect();
    split(&runs, &reading_chars).unwrap_or_else(|| vec![RubySpan::with_reading(surface, reading)])
}

/// A stretch of a word: all kanji, or none of it.
struct Run<'a> {
    kanji: bool,
    text: &'a str,
}

fn runs(surface: &str) -> Vec<Run<'_>> {
    let mut runs: Vec<Run> = Vec::new();
    let mut start = 0;
    let mut kanji = None;
    for (i, c) in surface.char_indices() {
        let is = is_kanji(c);
        if kanji.is_some_and(|k| k != is) {
            runs.push(Run {
                kanji: !is,
                text: &surface[start..i],
            });
            start = i;
        }
        kanji = Some(is);
    }
    if let Some(kanji) = kanji {
        runs.push(Run {
            kanji,
            text: &surface[start..],
        });
    }
    runs
}

/// Lays `reading` over `runs`: a kana run must be the next kana of the
/// reading, a kanji run takes what lies before the kana run that follows it
/// (at least one kana), or the rest. `None` when it cannot be done.
fn split(runs: &[Run], reading: &[char]) -> Option<Vec<RubySpan>> {
    let Some((run, rest)) = runs.split_first() else {
        return reading.is_empty().then(Vec::new);
    };
    if !run.kanji {
        let kana: Vec<char> = run.text.chars().map(hiragana).collect();
        let tail = reading.strip_prefix(kana.as_slice())?;
        let mut spans = vec![RubySpan::plain(run.text)];
        spans.extend(split(rest, tail)?);
        return Some(spans);
    }
    if rest.is_empty() {
        return (!reading.is_empty()).then(|| {
            vec![RubySpan::with_reading(
                run.text,
                reading.iter().collect::<String>(),
            )]
        });
    }
    // A kanji run is followed by a kana run (runs alternate): find where that
    // kana starts in the reading, leftmost first, and back off if the rest
    // does not fit.
    let kana: Vec<char> = rest[0].text.chars().map(hiragana).collect();
    (1..reading.len()).find_map(|at| {
        reading[at..].starts_with(&kana).then(|| {
            let mut spans = vec![RubySpan::with_reading(
                run.text,
                reading[..at].iter().collect::<String>(),
            )];
            spans.extend(split(rest, &reading[at..])?);
            Some(spans)
        })?
    })
}

/// Adds `span`, joining it to the span before it when both are plain.
fn push(spans: &mut Vec<RubySpan>, span: RubySpan) {
    match spans.last_mut() {
        Some(last) if last.reading.is_none() && span.reading.is_none() => {
            last.text.push_str(&span.text)
        }
        _ => spans.push(span),
    }
}

fn is_numeral(c: char) -> bool {
    "一二三四五六七八九十百千万".contains(c)
}

fn is_kanji(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '々')
}

fn is_hiragana_or_mark(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | 'ー')
}

/// Katakana as hiragana; every other character as it is.
fn hiragana(c: char) -> char {
    match c {
        '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
        _ => c,
    }
}

fn to_hiragana(text: &str) -> String {
    text.chars().map(hiragana).collect()
}

/// The analyser's words laid over `text` by their byte ranges, with whatever
/// the analyser skipped between them (it drops spaces) put back as plain words.
/// `None` when the ranges are out of order, overlap, or fall outside `text`.
fn restore_dropped(
    text: &str,
    found: Vec<(usize, usize, Option<String>)>,
) -> Option<Vec<(String, Option<String>)>> {
    let mut words = Vec::with_capacity(found.len());
    let mut at = 0;
    for (start, end, reading) in found {
        if start < at || end < start {
            return None;
        }
        if start > at {
            words.push((text.get(at..start)?.to_string(), None));
        }
        words.push((text.get(start..end)?.to_string(), reading));
        at = end;
    }
    if at < text.len() {
        words.push((text.get(at..)?.to_string(), None));
    }
    Some(words)
}

/// The analyser: lindera with the IPADIC dictionary embedded in the binary.
pub struct Furigana {
    segmenter: Segmenter,
}

impl Furigana {
    pub fn new() -> Result<Self, String> {
        let dictionary = load_dictionary("embedded://ipadic").map_err(|e| e.to_string())?;
        Ok(Self {
            segmenter: Segmenter::new(Mode::Normal, dictionary, None),
        })
    }

    /// `text` as [`RubySpan`]s. A line that cannot be analysed (or has no
    /// Japanese in it) comes back as one plain span.
    pub fn annotate(&self, text: &str) -> Vec<RubySpan> {
        if text.is_empty() {
            return Vec::new();
        }
        if !contains_japanese(text) {
            return vec![RubySpan::plain(text)];
        }
        let mut tokens = match self.segmenter.segment(Cow::Borrowed(text)) {
            Ok(tokens) => tokens,
            Err(e) => {
                log::warn!("furigana: could not analyse a line: {e}");
                return vec![RubySpan::plain(text)];
            }
        };
        // IPADIC details: ..., [7] reading (katakana), [8] pronunciation.
        let found = tokens
            .iter_mut()
            .map(|token| {
                let reading = token.details().get(7).map(|r| r.to_string());
                (token.byte_start, token.byte_end, reading)
            })
            .collect();
        // The analyser drops spaces: they are put back from the tokens' byte
        // ranges. A line that still does not read back as itself (ranges that
        // do not fit, a changed surface) is shown plain.
        let Some(words) = restore_dropped(text, found) else {
            return vec![RubySpan::plain(text)];
        };
        spans_from_tokens(words.iter().map(|(s, r)| (s.as_str(), r.as_deref())))
    }
}

/// [`Furigana::annotate`] with one analyser for the whole process, loaded on
/// first use. Without a working dictionary every line is plain.
pub fn annotate(text: &str) -> Vec<RubySpan> {
    static ANALYSER: OnceLock<Option<Furigana>> = OnceLock::new();
    let analyser = ANALYSER.get_or_init(|| match Furigana::new() {
        Ok(analyser) => Some(analyser),
        Err(e) => {
            log::error!("furigana dictionary did not load: {e}");
            None
        }
    });
    match analyser {
        Some(analyser) => analyser.annotate(text),
        None if text.is_empty() => Vec::new(),
        None => vec![RubySpan::plain(text)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(tokens: &[Token]) -> Vec<RubySpan> {
        spans_from_tokens(tokens.iter().copied())
    }

    fn r(text: &str, reading: &str) -> RubySpan {
        RubySpan::with_reading(text, reading)
    }

    fn p(text: &str) -> RubySpan {
        RubySpan::plain(text)
    }

    #[test]
    fn each_word_with_kanji_gets_its_reading_in_hiragana() {
        assert_eq!(
            spans(&[
                ("日", Some("ニチ")),
                ("韓", Some("カン")),
                ("辞書", Some("ジショ"))
            ]),
            [r("日", "にち"), r("韓", "かん"), r("辞書", "じしょ")]
        );
    }

    #[test]
    fn a_word_without_a_reading_is_read_kanji_by_kanji_in_on_yomi() {
        // 巨塔 is not in the dictionary: the analyser gives no reading.
        assert_eq!(spans(&[("巨塔", None)]), [r("巨", "きょ"), r("塔", "とう")]);
        assert_eq!(
            spans(&[("巨塔", Some("*"))]),
            [r("巨", "きょ"), r("塔", "とう")]
        );
    }

    #[test]
    fn the_on_yomi_fallback_leaves_kana_and_kanji_without_on_yomi_plain() {
        // Kana in an unknown word stay plain and merge with their neighbours;
        // 畑 has no on'yomi, so it stays plain too.
        assert_eq!(
            spans(&[("巨塔化け", None)]),
            [r("巨", "きょ"), r("塔", "とう"), r("化", "か"), p("け")]
        );
        assert_eq!(spans(&[("畑", None)]), [p("畑")]);
        assert_eq!(
            spans(&[("巨畑", None), ("だ", None)]),
            [r("巨", "きょ"), p("畑だ")]
        );
    }

    #[test]
    fn a_word_with_a_reading_does_not_use_the_on_yomi_fallback() {
        // 山 is known as やま: the analyser's reading wins over さん.
        assert_eq!(spans(&[("山", Some("ヤマ"))]), [r("山", "やま")]);
    }

    #[test]
    fn single_kanji_words_side_by_side_are_read_in_on_yomi() {
        // 響奏 is no Japanese word: the dictionary has 響 (ひびき, a name) and
        // 奏 (そう) separately, and ひびきそう mixes kun and on. Read as a
        // Sino-Japanese pair instead.
        assert_eq!(
            spans(&[("響", Some("ヒビキ")), ("奏", Some("ソウ"))]),
            [r("響", "きょう"), r("奏", "そう")]
        );
        // Longer runs too; a kanji with no on'yomi keeps its own reading.
        assert_eq!(
            spans(&[
                ("泡", Some("アワ")),
                ("影", Some("カゲ")),
                ("畑", Some("ハタケ"))
            ]),
            [r("泡", "ほう"), r("影", "えい"), r("畑", "はたけ")]
        );
    }

    #[test]
    fn a_run_that_starts_with_a_numeral_is_a_count_and_keeps_the_dictionary_readings() {
        // 三人 is さんにん: 人 after a number is the counter にん, not じん.
        assert_eq!(
            spans(&[("三", Some("サン")), ("人", Some("ニン"))]),
            [r("三", "さん"), r("人", "にん")]
        );
        // A numeral inside a run does not make it a count.
        assert_eq!(
            spans(&[("響", Some("ヒビキ")), ("一", Some("イチ"))]),
            [r("響", "きょう"), r("一", "いち")]
        );
    }

    #[test]
    fn one_person_and_two_people_are_read_as_whole_words() {
        // The dictionary gives 一 (いち) and 人 (にん) apart; nobody says いちにん.
        assert_eq!(
            spans(&[("一", Some("イチ")), ("人", Some("ニン"))]),
            [r("一人", "ひとり")]
        );
        assert_eq!(
            spans(&[("二", Some("ニ")), ("人", Some("ニン")), ("で", Some("デ"))]),
            [r("二人", "ふたり"), p("で")]
        );
        // Only at the start of the count: 十二人 is じゅうににん.
        assert_eq!(
            spans(&[
                ("十", Some("ジュウ")),
                ("二", Some("ニ")),
                ("人", Some("ニン"))
            ]),
            [r("十", "じゅう"), r("二", "に"), r("人", "にん")]
        );
        // 一人前 (一 + 人前) is いちにんまえ: 人前 is not the counter.
        assert_eq!(
            spans(&[("一", Some("イチ")), ("人前", Some("ニンマエ"))]),
            [r("一", "いち"), r("人前", "にんまえ")]
        );
    }

    #[test]
    fn a_single_kanji_word_on_its_own_or_next_to_other_words_keeps_its_reading() {
        // Alone, or with kana or a longer word between/after: not a run.
        assert_eq!(
            spans(&[
                ("響", Some("ヒビキ")),
                ("の", Some("ノ")),
                ("奏", Some("ソウ"))
            ]),
            [r("響", "ひびき"), p("の"), r("奏", "そう")]
        );
        assert_eq!(
            spans(&[("響", Some("ヒビキ")), ("奏者", Some("ソウシャ"))]),
            [r("響", "ひびき"), r("奏者", "そうしゃ")]
        );
        assert_eq!(spans(&[("山", Some("ヤマ"))]), [r("山", "やま")]);
    }

    #[test]
    fn what_the_analyser_dropped_between_words_is_put_back_plain() {
        let w = |s: &str, r: Option<&str>| (s.to_string(), r.map(str::to_string));
        // "墓M6 @D ５周": the spaces (bytes 5 and 8) are not in any range.
        let text = "墓M6 @D ５周";
        let found = vec![
            (0, 3, Some("ハカ".to_string())),
            (3, 4, None),
            (4, 5, None),
            (6, 7, None),
            (7, 8, None),
            (9, 12, Some("ゴ".to_string())),
            (12, 15, Some("シュウ".to_string())),
        ];
        assert_eq!(
            restore_dropped(text, found),
            Some(vec![
                w("墓", Some("ハカ")),
                w("M", None),
                w("6", None),
                w(" ", None),
                w("@", None),
                w("D", None),
                w(" ", None),
                w("５", Some("ゴ")),
                w("周", Some("シュウ")),
            ])
        );
        // A skipped tail is put back too; nothing skipped changes nothing.
        assert_eq!(
            restore_dropped("墓 ", vec![(0, 3, Some("ハカ".to_string()))]),
            Some(vec![w("墓", Some("ハカ")), w(" ", None)])
        );
        assert_eq!(restore_dropped("", vec![]), Some(vec![]));
    }

    #[test]
    fn ranges_that_do_not_fit_the_text_are_refused() {
        let r = |a, b| (a, b, None);
        assert_eq!(restore_dropped("abc", vec![r(2, 3), r(0, 1)]), None); // out of order
        assert_eq!(restore_dropped("abc", vec![r(0, 2), r(1, 3)]), None); // overlap
        assert_eq!(restore_dropped("abc", vec![r(0, 9)]), None); // past the end
        assert_eq!(restore_dropped("墓", vec![r(0, 2)]), None); // inside a character
    }

    #[test]
    fn okurigana_stays_plain_and_neighbouring_plain_pieces_merge() {
        assert_eq!(
            spans(&[("走っ", Some("ハシッ")), ("た", Some("タ"))]),
            [r("走", "はし"), p("った")]
        );
    }

    #[test]
    fn a_word_with_kana_between_its_kanji_splits_on_them() {
        assert_eq!(
            spans(&[("取り込む", Some("トリコム"))]),
            [r("取", "と"), p("り"), r("込", "こ"), p("む")]
        );
        assert_eq!(
            spans(&[("思い出す", Some("オモイダス"))]),
            [r("思", "おも"), p("い"), r("出", "だ"), p("す")]
        );
    }

    #[test]
    fn a_leading_kana_is_left_plain() {
        assert_eq!(
            spans(&[("お疲れ様", Some("オツカレサマ"))]),
            [p("お"), r("疲", "つか"), p("れ"), r("様", "さま")]
        );
    }

    #[test]
    fn words_without_kanji_or_without_a_reading_are_plain_and_merge() {
        assert_eq!(
            spans(&[
                ("パーティー", Some("パーティー")),
                ("を", Some("ヲ")),
                ("22", None),
                ("、", None),
            ]),
            [p("パーティーを22、")]
        );
        // The analyser did not know the word (empty or "*" counts as no
        // reading): it is read kanji by kanji, see
        // `a_word_without_a_reading_is_read_kanji_by_kanji_in_on_yomi`.
        assert_eq!(
            spans(&[("深淵", Some(""))]),
            [r("深", "しん"), r("淵", "えん")]
        );
    }

    #[test]
    fn the_iteration_mark_belongs_to_the_kanji_before_it() {
        assert_eq!(
            spans(&[("人々", Some("ヒトビト"))]),
            [r("人々", "ひとびと")]
        );
    }

    #[test]
    fn a_word_that_does_not_line_up_gets_one_reading_over_all_of_it() {
        // ヶ is read か here: the kana of the word and its reading disagree.
        assert_eq!(
            spans(&[("一ヶ月", Some("イッカゲツ"))]),
            [r("一ヶ月", "いっかげつ")]
        );
        // Anchor kana the reading does not contain.
        assert_eq!(spans(&[("食べる", Some("ノム"))]), [r("食べる", "のむ")]);
    }

    #[test]
    fn the_spans_always_read_back_as_the_line_and_only_kanji_carry_readings() {
        let tokens: &[Token] = &[
            ("お疲れ様", Some("オツカレサマ")),
            ("です", Some("デス")),
            ("、", None),
            ("取り込む", Some("トリコム")),
            ("一ヶ月", Some("イッカゲツ")),
            ("22", None),
            ("人々", Some("ヒトビト")),
            ("深淵", None),
            ("ありがとう", Some("アリガトウ")),
        ];
        let line: String = tokens.iter().map(|(surface, _)| *surface).collect();
        let got = spans(tokens);
        assert_eq!(
            got.iter().map(|s| s.text.as_str()).collect::<String>(),
            line
        );
        for span in &got {
            assert!(!span.text.is_empty());
            if let Some(reading) = &span.reading {
                assert!(span.text.chars().any(is_kanji), "{span:?}");
                assert!(!reading.is_empty());
                assert!(reading.chars().all(is_hiragana_or_mark), "{span:?}");
            }
        }
    }

    #[test]
    fn no_tokens_is_no_spans() {
        assert_eq!(spans(&[]), []);
    }

    fn is_kanji(c: char) -> bool {
        matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '々')
    }

    fn is_hiragana_or_mark(c: char) -> bool {
        matches!(c, '\u{3041}'..='\u{3096}' | 'ー')
    }

    // The dictionary-backed tests below load the real IPADIC.

    fn analyser() -> Furigana {
        Furigana::new().expect("the embedded dictionary loads")
    }

    #[test]
    fn a_line_is_read_word_by_word_from_the_dictionary() {
        let f = analyser();
        assert_eq!(
            f.annotate("日韓辞書"),
            [r("日", "にち"), r("韓", "かん"), r("辞書", "じしょ")]
        );
        assert_eq!(
            f.annotate("今日はパーティー募集します"),
            [
                r("今日", "きょう"),
                p("はパーティー"),
                r("募集", "ぼしゅう"),
                p("します")
            ]
        );
        assert_eq!(
            f.annotate("明日22時から"),
            [r("明日", "あした"), p("22"), r("時", "じ"), p("から")]
        );
    }

    #[test]
    fn a_word_the_dictionary_lacks_is_read_in_on_yomi() {
        let f = analyser();
        // (No space in the line: some lindera builds drop spaces, and a line
        // that does not read back as itself is shown plain.)
        assert_eq!(
            f.annotate("巨塔M6"),
            [r("巨", "きょ"), r("塔", "とう"), p("M6")]
        );
    }

    #[test]
    fn two_kanji_the_dictionary_only_knows_apart_are_read_in_on_yomi() {
        let f = analyser();
        assert_eq!(f.annotate("響奏"), [r("響", "きょう"), r("奏", "そう")]);
        assert_eq!(
            f.annotate("響奏の力"),
            [
                r("響", "きょう"),
                r("奏", "そう"),
                p("の"),
                r("力", "ちから")
            ]
        );
    }

    #[test]
    fn people_are_counted_the_way_they_are_said() {
        // Roadmap K16: the built exe read 一人 as いち・じん.
        let f = analyser();
        assert_eq!(f.annotate("一人"), [r("一人", "ひとり")]);
        assert_eq!(
            f.annotate("二人で行きます"),
            [r("二人", "ふたり"), p("で"), r("行", "い"), p("きます")]
        );
        assert_eq!(f.annotate("三人"), [r("三", "さん"), r("人", "にん")]);
        assert_eq!(
            f.annotate("一人前"),
            [r("一", "いち"), r("人前", "にんまえ")]
        );
    }

    #[test]
    fn a_line_with_spaces_is_still_read() {
        // The analyser drops spaces; the line must not come back plain for it.
        let f = analyser();
        assert_eq!(
            f.annotate("墓M6 @D ５周"),
            [r("墓", "はか"), p("M6 @D ５"), r("周", "しゅう")]
        );
        assert_eq!(
            f.annotate("巨塔M6 5周 @D"),
            [
                r("巨", "きょ"),
                r("塔", "とう"),
                p("M6 5"),
                r("周", "しゅう"),
                p(" @D")
            ]
        );
    }

    #[test]
    fn a_line_without_kanji_is_one_plain_span_and_nothing_is_nothing() {
        let f = analyser();
        assert_eq!(f.annotate("ありがとう!"), [p("ありがとう!")]);
        assert_eq!(f.annotate("[스티커]よろしく"), [p("[스티커]よろしく")]);
        assert_eq!(f.annotate("hello"), [p("hello")]);
        assert_eq!(f.annotate(""), []);
    }

    #[test]
    fn a_real_line_reads_back_as_itself() {
        let f = analyser();
        for line in [
            "お疲れ様です、周回手伝ってください",
            "明日22時から深淵行く人いますか",
            "@abc 募集 [스티커] 大人気の剣士",
            "ｱｲﾃﾑ　全角スペースと半角 space",
        ] {
            let got = f.annotate(line);
            assert_eq!(
                got.iter().map(|s| s.text.as_str()).collect::<String>(),
                line
            );
            assert!(got.iter().all(|s| !s.text.is_empty()), "{got:?}");
        }
    }

    #[test]
    fn the_shared_analyser_answers_like_a_fresh_one() {
        assert_eq!(annotate("日韓辞書"), analyser().annotate("日韓辞書"));
    }
}
