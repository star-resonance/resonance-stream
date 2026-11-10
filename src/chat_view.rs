//! Which chat messages a view shows. Pure functions, so they are tested on
//! the host (`cargo test -p resonance-stream-ui`) without a browser.

use crate::ui_types::{default_tab_limit, Channel, ChatMessage};
use std::collections::{HashMap, VecDeque};

pub use crate::ui_types::{ALL_TAB, CUSTOM_TAB, SYSTEM_TAB};

/// What a tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    All,
    Custom,
    System,
    Channel(Channel),
}

impl Tab {
    /// The tabs in the order the nav bar lists them (System only in debug mode).
    pub fn nav(debug_mode: bool) -> Vec<Tab> {
        let mut tabs = vec![
            Tab::All,
            Tab::Custom,
            Tab::Channel(Channel::World),
            Tab::Channel(Channel::Guild),
            Tab::Channel(Channel::Party),
            Tab::Channel(Channel::Local),
            Tab::Channel(Channel::Beginner),
        ];
        if debug_mode {
            tabs.push(Tab::System);
        }
        tabs
    }

    /// The tabs the tab-switch shortcut cycles through, in order.
    const SWITCH_ORDER: [Tab; 6] = [
        Tab::Custom,
        Tab::Channel(Channel::World),
        Tab::Channel(Channel::Guild),
        Tab::Channel(Channel::Party),
        Tab::Channel(Channel::Local),
        Tab::Channel(Channel::Beginner),
    ];

    /// The tab the shortcut switches to from the tab labelled `current`: the
    /// next in [`SWITCH_ORDER`](Self::SWITCH_ORDER), or the custom tab when
    /// `current` is not in it (the all-tab, the system tab).
    pub fn switch_from(current: &str) -> Tab {
        match Self::SWITCH_ORDER
            .iter()
            .position(|tab| tab.label() == current)
        {
            Some(i) => Self::SWITCH_ORDER[(i + 1) % Self::SWITCH_ORDER.len()],
            None => Tab::Custom,
        }
    }

    /// The tab labelled `label` (how `active_tab` is stored), if there is one.
    pub fn parse(label: &str) -> Option<Self> {
        match label {
            ALL_TAB => Some(Tab::All),
            CUSTOM_TAB => Some(Tab::Custom),
            SYSTEM_TAB => Some(Tab::System),
            other => Channel::from_label(other).map(Tab::Channel),
        }
    }

    /// Like [`parse`](Self::parse); a label nobody knows is the world tab.
    pub fn from_label(label: &str) -> Self {
        Self::parse(label).unwrap_or(Tab::Channel(Channel::World))
    }

    /// The tab's label: what the nav bar shows and `active_tab` stores.
    pub fn label(self) -> &'static str {
        match self {
            Tab::All => ALL_TAB,
            Tab::Custom => CUSTOM_TAB,
            Tab::System => SYSTEM_TAB,
            Tab::Channel(channel) => channel.label(),
        }
    }

    /// The tab's key in `tab_limits`, the unread counts and the context menu:
    /// its label, or for a channel its name; `"SYSTEM"` for the system tab.
    pub fn key(self) -> &'static str {
        match self {
            Tab::All => ALL_TAB,
            Tab::Custom => CUSTOM_TAB,
            Tab::System => "SYSTEM",
            Tab::Channel(channel) => channel.as_str(),
        }
    }

    /// The tab's colour dot in the tab bar.
    pub fn dot_class(self) -> &'static str {
        match self {
            Tab::All => "bg-base-content/60",
            Tab::Custom => "bg-slate-400",
            Tab::System => "bg-warning",
            Tab::Channel(Channel::World) => "bg-purple-500",
            Tab::Channel(Channel::Guild) => "bg-emerald-500",
            Tab::Channel(Channel::Party) => "bg-sky-500",
            Tab::Channel(Channel::Local) => "bg-base-content/30",
            Tab::Channel(Channel::Beginner) => "bg-amber-500",
        }
    }

    /// Does the right-click menu have the "save to disk" switch? (Channels only.)
    pub fn has_archive_setting(self) -> bool {
        matches!(self, Tab::Channel(_))
    }

    /// Does a message on `channel` belong to this tab? (No level or search
    /// filtering: this decides unread badges.)
    pub fn shows_channel(self, channel: Channel, custom_filters: &[String]) -> bool {
        match self {
            Tab::All => true,
            Tab::Custom => custom_filters.iter().any(|c| c == channel.as_str()),
            Tab::System => false,
            Tab::Channel(own) => channel == own,
        }
    }

    /// Drops the unread counts that opening this tab clears: all of them for
    /// the all-tab, the selected channels' for the custom tab, else its own.
    pub fn clear_unread(self, counts: &mut HashMap<String, usize>, custom_filters: &[String]) {
        match self {
            Tab::All => counts.clear(),
            Tab::Custom => {
                for channel in custom_filters {
                    counts.remove(channel);
                }
            }
            Tab::System | Tab::Channel(_) => {
                counts.remove(self.key());
            }
        }
    }

    /// Key of the tab's message list in [`TabViews`] (and in `tab_limits`);
    /// the system tab has no list.
    pub fn view_key(self) -> Option<&'static str> {
        (self != Tab::System).then(|| self.key())
    }
}

/// The tabs the user may show or hide, in the bar's order: every tab but
/// the system tab (debug mode decides that one).
const EDITABLE_TABS: [Tab; 7] = [
    Tab::All,
    Tab::Custom,
    Tab::Channel(Channel::World),
    Tab::Channel(Channel::Guild),
    Tab::Channel(Channel::Party),
    Tab::Channel(Channel::Local),
    Tab::Channel(Channel::Beginner),
];

fn is_chosen(nav_tabs: &[String], tab: Tab) -> bool {
    nav_tabs.iter().any(|label| label == tab.label())
}

/// The tabs the nav bar shows: the chosen ones (`nav_tabs`, by label) in the
/// bar's order, then the system tab in debug mode. None chosen (a hand-edited
/// file): every tab.
pub fn shown_tabs(nav_tabs: &[String], debug_mode: bool) -> Vec<Tab> {
    let mut tabs: Vec<Tab> = EDITABLE_TABS
        .into_iter()
        .filter(|&tab| is_chosen(nav_tabs, tab))
        .collect();
    if tabs.is_empty() {
        tabs = EDITABLE_TABS.to_vec();
    }
    if debug_mode {
        tabs.push(Tab::System);
    }
    tabs
}

/// The tabs the (+) button offers: the ones the bar does not show.
pub fn hidden_tabs(nav_tabs: &[String]) -> Vec<Tab> {
    let shown = shown_tabs(nav_tabs, false);
    EDITABLE_TABS
        .into_iter()
        .filter(|tab| !shown.contains(tab))
        .collect()
}

pub fn show_tab(nav_tabs: &mut Vec<String>, tab: Tab) {
    if tab != Tab::System && !is_chosen(nav_tabs, tab) {
        nav_tabs.push(tab.label().to_string());
    }
}

/// Hides `tab`; false (and nothing changes) for the last shown tab or the
/// system tab.
pub fn hide_tab(nav_tabs: &mut Vec<String>, tab: Tab) -> bool {
    let shown = shown_tabs(nav_tabs, false);
    if tab == Tab::System || !shown.contains(&tab) || shown.len() <= 1 {
        return false;
    }
    // Store what is shown, so an empty or odd list becomes a real one.
    *nav_tabs = shown
        .into_iter()
        .filter(|&t| t != tab)
        .map(|t| t.label().to_string())
        .collect();
    true
}

/// The tab to show for the stored active tab `active`: itself while the bar
/// shows it, else the bar's first tab.
pub fn active_shown(active: &str, nav_tabs: &[String], debug_mode: bool) -> Tab {
    let shown = shown_tabs(nav_tabs, debug_mode);
    Tab::parse(active)
        .filter(|tab| shown.contains(tab))
        .unwrap_or(shown[0])
}

/// The tab the tab-switch shortcut goes to from `current`: the next shown
/// tab of [`Tab::SWITCH_ORDER`], its first one from a tab outside it; the
/// bar's first tab when it shows none of them.
pub fn switch_shown(current: &str, nav_tabs: &[String]) -> Tab {
    let shown = shown_tabs(nav_tabs, false);
    let cycle: Vec<Tab> = Tab::SWITCH_ORDER
        .into_iter()
        .filter(|tab| shown.contains(tab))
        .collect();
    if cycle.is_empty() {
        return shown[0];
    }
    match cycle.iter().position(|tab| tab.label() == current) {
        Some(i) => cycle[(i + 1) % cycle.len()],
        None => cycle[0],
    }
}

/// How many messages a tab keeps. Channel tabs have their own limit
/// (right-click menu; unset: `default_tab_limit`). The all-tab and the
/// custom tab have no input of their own: the all-tab holds as many as all
/// channel limits together (2000 when none is set), the custom tab as many
/// as its selected channels together.
pub fn tab_limit(limits: &HashMap<String, usize>, key: &str, custom_filters: &[String]) -> usize {
    let own = |key: &str| {
        limits
            .get(key)
            .copied()
            .unwrap_or_else(|| default_tab_limit(key))
    };
    let limit = match key {
        ALL_TAB => {
            let sum: usize = limits
                .iter()
                .filter(|(k, _)| {
                    !matches!(k.as_str(), ALL_TAB | CUSTOM_TAB) && k.as_str() != Tab::System.key()
                })
                .map(|(_, v)| *v)
                .sum();
            if sum == 0 {
                2000
            } else {
                sum
            }
        }
        CUSTOM_TAB => custom_filters.iter().map(|channel| own(channel)).sum(),
        _ => own(key),
    };
    limit.max(1)
}

/// Per-tab message lists (pids, oldest first), each capped by its own limit,
/// so a busy world chat cannot push party or guild messages out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TabViews {
    views: HashMap<String, VecDeque<u64>>,
    /// How many views hold each pid; a pid no view holds can be dropped.
    refs: HashMap<u64, usize>,
}

impl TabViews {
    /// Files a new message under the all-tab, its channel and (when
    /// selected) the custom tab. Returns the pids no tab holds any more.
    pub fn add(
        &mut self,
        pid: u64,
        channel: Channel,
        custom_filters: &[String],
        limits: &HashMap<String, usize>,
    ) -> Vec<u64> {
        let mut keys = vec![ALL_TAB, channel.as_str()];
        if custom_filters.iter().any(|c| c == channel.as_str()) {
            keys.push(CUSTOM_TAB);
        }
        let mut dropped = Vec::new();
        for key in keys {
            let limit = tab_limit(limits, key, custom_filters);
            let view = self.views.entry(key.to_string()).or_default();
            view.push_back(pid);
            *self.refs.entry(pid).or_insert(0) += 1;
            while view.len() > limit {
                let Some(old) = view.pop_front() else { break };
                if let Some(count) = self.refs.get_mut(&old) {
                    *count -= 1;
                    if *count == 0 {
                        self.refs.remove(&old);
                        dropped.push(old);
                    }
                }
            }
        }
        dropped
    }

    /// The tab's pids, oldest first.
    pub fn pids<'a>(&'a self, key: &str) -> impl DoubleEndedIterator<Item = u64> + 'a {
        self.views.get(key).into_iter().flatten().copied()
    }

    pub fn len(&self, key: &str) -> usize {
        self.views.get(key).map_or(0, VecDeque::len)
    }

    pub fn clear(&mut self) {
        self.views.clear();
        self.refs.clear();
    }
}

/// The chat messages the UI holds: each by pid, plus the per-tab lists.
/// A message is kept exactly as long as some tab lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatStore<T> {
    messages: HashMap<u64, T>,
    views: TabViews,
    /// Translations that arrived before their row did (the history fetch had
    /// not delivered it yet): `pid -> text`, handed back after the merge.
    held: HashMap<u64, String>,
}

/// How many such translations are kept; the oldest pid goes first.
pub const HELD_TRANSLATIONS_MAX: usize = 256;

impl<T> Default for ChatStore<T> {
    fn default() -> Self {
        Self {
            messages: HashMap::new(),
            views: TabViews::default(),
            held: HashMap::new(),
        }
    }
}

impl<T: Clone> ChatStore<T> {
    pub fn add(
        &mut self,
        pid: u64,
        channel: Channel,
        message: T,
        custom_filters: &[String],
        limits: &HashMap<String, usize>,
    ) {
        self.messages.insert(pid, message);
        for gone in self.views.add(pid, channel, custom_filters, limits) {
            self.messages.remove(&gone);
        }
    }

    pub fn get(&self, pid: u64) -> Option<&T> {
        self.messages.get(&pid)
    }

    /// Keeps a translation aside whose row is not in the store (yet). A row
    /// already held needs no help: the caller updates it directly.
    pub fn hold_translation(&mut self, pid: u64, text: String) {
        if self.messages.contains_key(&pid) {
            return;
        }
        self.held.insert(pid, text);
        while self.held.len() > HELD_TRANSLATIONS_MAX {
            let Some(oldest) = self.held.keys().min().copied() else {
                break;
            };
            self.held.remove(&oldest);
        }
    }

    /// The held translations whose row has arrived since, to apply; the rest
    /// are dropped (their row was evicted or never came).
    pub fn take_held_translations(&mut self) -> Vec<(u64, String)> {
        let mut held: Vec<_> = std::mem::take(&mut self.held)
            .into_iter()
            .filter(|(pid, _)| self.messages.contains_key(pid))
            .collect();
        held.sort_unstable_by_key(|(pid, _)| *pid);
        held
    }

    /// Merges the backend's history (`(pid, channel, message)`) under the
    /// messages that arrived live while it was being fetched, instead of
    /// replacing them. Pids give the order (the backend hands them out in
    /// arrival order); a pid held both ways keeps the live copy, which
    /// events may already have updated.
    pub fn merge_history(
        &mut self,
        history: Vec<(u64, Channel, T)>,
        channel_of: impl Fn(&T) -> Channel,
        custom_filters: &[String],
        limits: &HashMap<String, usize>,
    ) {
        let mut live = std::mem::take(self);
        self.held = std::mem::take(&mut live.held);
        let mut rows: Vec<(u64, Channel, T)> = history
            .into_iter()
            .filter(|(pid, _, _)| !live.messages.contains_key(pid))
            .collect();
        rows.extend(
            live.messages
                .into_iter()
                .map(|(pid, m)| (pid, channel_of(&m), m)),
        );
        rows.sort_unstable_by_key(|(pid, _, _)| *pid);
        for (pid, channel, message) in rows {
            self.add(pid, channel, message, custom_filters, limits);
        }
    }

    /// The tab's messages, oldest first.
    pub fn tab<'a>(&'a self, key: &str) -> impl DoubleEndedIterator<Item = T> + 'a {
        self.views
            .pids(key)
            .filter_map(move |pid| self.messages.get(&pid).cloned())
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Every message held, in arrival (pid) order.
    pub fn all(&self) -> Vec<T> {
        let mut pids: Vec<_> = self.messages.keys().copied().collect();
        pids.sort_unstable();
        pids.iter().map(|pid| self.messages[pid].clone()).collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.views.clear();
        self.held.clear();
    }
}

/// The chat list's filter: tab, minimum sender level (world chat only) and
/// search text (nickname or message, case-insensitive).
pub struct ChatFilter<'a> {
    tab: Tab,
    custom_filters: &'a [String],
    min_level: u64,
    search_lower: String,
}

impl<'a> ChatFilter<'a> {
    pub fn new(tab: Tab, custom_filters: &'a [String], min_level: u64, search: &str) -> Self {
        Self {
            tab,
            custom_filters,
            min_level,
            search_lower: search.to_lowercase(),
        }
    }

    pub fn matches(&self, m: &ChatMessage) -> bool {
        if !self.tab.shows_channel(m.channel, self.custom_filters) {
            return false;
        }
        if m.channel == Channel::World && m.level < self.min_level {
            return false;
        }
        self.search_lower.is_empty()
            || m.nickname.to_lowercase().contains(&self.search_lower)
            || m.message.to_lowercase().contains(&self.search_lower)
    }
}

/// A message that neither pings (keyword alert) nor counts as unread: a
/// blocked sender, or WORLD chat below the minimum sender level.
pub fn is_muted(m: &ChatMessage, min_level: u64) -> bool {
    m.is_blocked || (m.channel == Channel::World && m.level < min_level)
}

/// How long ago a message was sent, as a row shows it with relative time on: `now` (under 10 s), `42s`,
/// `5m`, `2h 10m`, `3d`. Both stamps are seconds, or milliseconds when past 10^10 (an older log kept
/// milliseconds); a message from the future reads `now`.
pub fn relative_time(message_ts: u64, now_ts: u64) -> String {
    let secs = |ts: u64| if ts > 10_000_000_000 { ts / 1000 } else { ts };
    let (message, now) = (secs(message_ts), secs(now_ts));
    let age = now.saturating_sub(message);
    if age < 10 {
        "now".to_string()
    } else if age < 60 {
        format!("{}s", age)
    } else if age < 3600 {
        format!("{}m", age / 60)
    } else if age < 86400 {
        let (hours, mins) = (age / 3600, (age % 3600) / 60);
        if mins > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}h", hours)
        }
    } else {
        format!("{}d", age / 86400)
    }
}

/// Rows the list shows to start with, and the number each scroll to the top adds.
pub const DISPLAY_PAGE: usize = 50;

/// How close to the top (in px) a scroll counts as reaching it.
pub const LOAD_MORE_ZONE: i32 = 50;

/// How many rows the list shows after a scroll event: one more page on reaching the top, and back to
/// the first page once the view is at the bottom again (P-1: the limit only ever grew, so the longer
/// the overlay stayed open the more rows the page held). At the bottom the older pages are not on
/// screen, so dropping them is invisible; a list shorter than the window is at the bottom too, and has
/// nothing older to load.
pub fn next_display_limit(limit: usize, scroll_top: i32, at_bottom: bool) -> usize {
    if at_bottom {
        DISPLAY_PAGE
    } else if scroll_top < LOAD_MORE_ZONE {
        limit + DISPLAY_PAGE
    } else {
        limit
    }
}

/// The newest `limit` items accepted by `keep`, oldest first -- walking
/// from the newest end, so a short page of a long list stops early.
pub fn newest_matching<T>(
    items: impl DoubleEndedIterator<Item = T>,
    limit: usize,
    mut keep: impl FnMut(&T) -> bool,
) -> Vec<T> {
    let mut page: Vec<T> = items.rev().filter(|v| keep(v)).take(limit).collect();
    page.reverse();
    page
}

/// Display classes of the compact-mode original line under its translation.
/// Exactly one bare `display` utility may be present: two (`block` +
/// `hidden`) are resolved by stylesheet order, not by the string -- the
/// original once stayed visible that way.
pub fn compact_original_class(hide_original: bool, has_translation: bool) -> &'static str {
    if hide_original && has_translation {
        "hidden group-hover:block"
    } else {
        "block"
    }
}

/// A line the translator will still answer: Japanese, not blocked, not yet
/// translated, translation on. The row shows a "..." for it.
pub fn translation_pending(m: &ChatMessage, use_translation: bool) -> bool {
    use_translation
        && !m.is_blocked
        && m.translated.is_none()
        && crate::ui_types::contains_japanese(&m.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_age_reads_now_seconds_minutes_hours_days() {
        // Moved out of `ChatRow` unchanged (timestamps in seconds).
        let at = |age: u64| relative_time(1_760_000_000, 1_760_000_000 + age);
        assert_eq!(at(0), "now");
        assert_eq!(at(9), "now");
        assert_eq!(at(10), "10s");
        assert_eq!(at(59), "59s");
        assert_eq!(at(60), "1m");
        assert_eq!(at(3599), "59m");
        assert_eq!(at(3600), "1h");
        assert_eq!(at(3660), "1h 1m");
        assert_eq!(at(86_399), "23h 59m");
        assert_eq!(at(86_400), "1d");
        assert_eq!(at(3 * 86_400 + 5), "3d");
    }

    #[test]
    fn a_message_from_the_future_reads_now() {
        assert_eq!(relative_time(1_760_000_100, 1_760_000_000), "now");
    }

    #[test]
    fn timestamps_in_milliseconds_are_read_as_seconds() {
        // The backend sends seconds, an older log had milliseconds: past 1e10 it is milliseconds.
        assert_eq!(relative_time(1_760_000_000_000, 1_760_000_090_000), "1m");
        assert_eq!(relative_time(1_760_000_000, 1_760_000_090_000), "1m");
        assert_eq!(relative_time(1_760_000_000_000, 1_760_000_090), "1m");
    }

    fn bare_display(class: &str) -> Vec<&str> {
        class
            .split_whitespace()
            .filter(|c| matches!(*c, "inline" | "hidden" | "block" | "flex"))
            .collect()
    }

    #[test]
    fn hidden_original_has_no_competing_display_class() {
        let c = compact_original_class(true, true);
        assert_eq!(bare_display(c), ["hidden"]);
        assert!(c.contains("group-hover:block"));
    }

    #[test]
    fn original_is_shown_without_hide_or_without_translation() {
        for (hide, tr) in [(false, true), (false, false), (true, false)] {
            assert_eq!(bare_display(compact_original_class(hide, tr)), ["block"]);
        }
    }

    #[test]
    fn each_tab_has_its_own_dot_colour() {
        let mut tabs = Tab::nav(true);
        tabs.dedup();
        let dots: Vec<_> = tabs.iter().map(|t| t.dot_class()).collect();
        for (tab, dot) in tabs.iter().zip(&dots) {
            assert!(dot.starts_with("bg-"), "{tab:?}: {dot}");
        }
        let mut unique = dots.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), dots.len(), "{dots:?}");
    }

    #[test]
    fn only_an_untranslated_japanese_line_waits_for_a_translation() {
        let mut m = msg(Channel::World, 50, "a", "こんにちは");
        assert!(translation_pending(&m, true));
        assert!(!translation_pending(&m, false), "translation switched off");
        m.translated = Some("안녕".into());
        assert!(!translation_pending(&m, true), "already translated");
        let mut blocked = msg(Channel::World, 50, "a", "こんにちは");
        blocked.is_blocked = true;
        assert!(
            !translation_pending(&blocked, true),
            "blocked lines are not shown"
        );
        assert!(!translation_pending(
            &msg(Channel::World, 50, "a", "안녕하세요"),
            true
        ));
    }

    fn msg(channel: Channel, level: u64, nickname: &str, message: &str) -> ChatMessage {
        ChatMessage {
            channel,
            level,
            nickname: nickname.into(),
            message: message.into(),
            ..Default::default()
        }
    }

    #[test]
    fn tab_labels() {
        assert_eq!(Tab::from_label("전체"), Tab::All);
        assert_eq!(Tab::from_label("커스텀"), Tab::Custom);
        assert_eq!(Tab::from_label("시스템"), Tab::System);
        assert_eq!(Tab::from_label("로컬"), Tab::Channel(Channel::Local));
        assert_eq!(Tab::from_label("파티"), Tab::Channel(Channel::Party));
        assert_eq!(Tab::from_label("길드"), Tab::Channel(Channel::Guild));
        assert_eq!(Tab::from_label("월드"), Tab::Channel(Channel::World));
        assert_eq!(Tab::from_label("초보자"), Tab::Channel(Channel::Beginner));
        assert_eq!(
            Tab::from_label("anything else"),
            Tab::Channel(Channel::World)
        );
    }

    #[test]
    fn the_nav_lists_the_tabs_in_their_order() {
        let labels: Vec<_> = Tab::nav(false).iter().map(|t| t.label()).collect();
        assert_eq!(
            labels,
            ["전체", "커스텀", "월드", "길드", "파티", "로컬", "초보자"]
        );
        let debug: Vec<_> = Tab::nav(true).iter().map(|t| t.label()).collect();
        assert_eq!(debug.last(), Some(&"시스템"));
        assert_eq!(debug.len(), 8);
    }

    #[test]
    fn a_tab_is_found_again_by_its_label_and_has_a_key() {
        for tab in Tab::nav(true) {
            assert_eq!(Tab::parse(tab.label()), Some(tab), "{tab:?}");
        }
        assert_eq!(Tab::parse("anything else"), None);
        let keys: Vec<_> = Tab::nav(true).iter().map(|t| t.key()).collect();
        assert_eq!(
            keys,
            [
                "전체",
                "커스텀",
                "WORLD",
                "GUILD",
                "PARTY",
                "LOCAL",
                "BEGINNER",
                "SYSTEM"
            ]
        );
        assert_eq!(Tab::System.view_key(), None);
        assert_eq!(Tab::Custom.view_key(), Some(CUSTOM_TAB));
        assert_eq!(Tab::Channel(Channel::Guild).view_key(), Some("GUILD"));
    }

    #[test]
    fn only_channel_tabs_have_the_archive_switch_and_their_own_dots() {
        let with: Vec<_> = Tab::nav(true)
            .into_iter()
            .filter(|t| t.has_archive_setting())
            .collect();
        assert_eq!(with.len(), 5);
        assert!(with.iter().all(|t| matches!(t, Tab::Channel(_))));
        // Gray, so it is not mistaken for the guild tab's green dot.
        assert_eq!(Tab::Custom.dot_class(), "bg-slate-400");
        assert_eq!(
            Tab::Channel(Channel::Local).dot_class(),
            "bg-base-content/30"
        );
    }

    #[test]
    fn the_tab_switch_shortcut_cycles_and_falls_back_to_custom() {
        let mut label = "커스텀";
        let mut seen = Vec::new();
        for _ in 0..7 {
            let next = Tab::switch_from(label);
            label = next.label();
            seen.push(label);
        }
        assert_eq!(
            seen,
            ["월드", "길드", "파티", "로컬", "초보자", "커스텀", "월드"]
        );
        for outside in ["전체", "시스템", "nonsense", ""] {
            assert_eq!(Tab::switch_from(outside), Tab::Custom, "{outside:?}");
        }
    }

    fn labels(tabs: &[Tab]) -> Vec<&'static str> {
        tabs.iter().map(|t| t.label()).collect()
    }

    fn nav(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_bar_shows_the_chosen_tabs_in_the_bars_order() {
        let defaults = resonance_types::default_nav_tabs();
        assert_eq!(shown_tabs(&defaults, false), Tab::nav(false));
        assert_eq!(shown_tabs(&defaults, true), Tab::nav(true));
        // Stored order and unknown labels do not matter; the system tab is debug-only.
        let chosen = nav(&["파티", "nonsense", "월드", "시스템"]);
        assert_eq!(labels(&shown_tabs(&chosen, false)), ["월드", "파티"]);
        assert_eq!(
            labels(&shown_tabs(&chosen, true)),
            ["월드", "파티", "시스템"]
        );
        // Nothing (a hand-edited file): every tab, never an empty bar.
        assert_eq!(shown_tabs(&[], false), Tab::nav(false));
    }

    #[test]
    fn the_plus_button_lists_the_hidden_tabs_and_adds_one_back() {
        let mut chosen = nav(&["월드", "길드", "파티"]);
        assert_eq!(
            labels(&hidden_tabs(&chosen)),
            ["전체", "커스텀", "로컬", "초보자"]
        );
        show_tab(&mut chosen, Tab::Channel(Channel::Local));
        show_tab(&mut chosen, Tab::Channel(Channel::Local));
        assert_eq!(
            labels(&shown_tabs(&chosen, false)),
            ["월드", "길드", "파티", "로컬"]
        );
        assert!(hidden_tabs(&resonance_types::default_nav_tabs()).is_empty());
    }

    #[test]
    fn a_tab_hides_unless_it_is_the_last_one() {
        let mut chosen = nav(&["전체", "길드"]);
        assert!(hide_tab(&mut chosen, Tab::All));
        assert_eq!(labels(&shown_tabs(&chosen, false)), ["길드"]);
        assert!(!hide_tab(&mut chosen, Tab::Channel(Channel::Guild)));
        assert_eq!(labels(&shown_tabs(&chosen, false)), ["길드"]);
        // The system tab is not the user's to hide.
        assert!(!hide_tab(&mut chosen, Tab::System));
    }

    #[test]
    fn a_hidden_active_tab_gives_way_to_the_first_shown_one() {
        let chosen = nav(&["길드", "파티"]);
        assert_eq!(
            active_shown("파티", &chosen, false),
            Tab::Channel(Channel::Party)
        );
        assert_eq!(
            active_shown("전체", &chosen, false),
            Tab::Channel(Channel::Guild)
        );
        assert_eq!(
            active_shown("시스템", &chosen, false),
            Tab::Channel(Channel::Guild)
        );
        assert_eq!(active_shown("시스템", &chosen, true), Tab::System);
    }

    #[test]
    fn the_tab_switch_shortcut_cycles_only_the_shown_tabs() {
        let chosen = nav(&["전체", "길드", "파티", "커스텀"]);
        assert_eq!(
            switch_shown("커스텀", &chosen),
            Tab::Channel(Channel::Guild)
        );
        assert_eq!(switch_shown("길드", &chosen), Tab::Channel(Channel::Party));
        assert_eq!(switch_shown("파티", &chosen), Tab::Custom);
        // From a tab outside the cycle (the all-tab, a hidden one): its first.
        assert_eq!(switch_shown("전체", &chosen), Tab::Custom);
        assert_eq!(switch_shown("월드", &chosen), Tab::Custom);
        // Only the all-tab shown: it stays.
        assert_eq!(switch_shown("전체", &nav(&["전체"])), Tab::All);
        // The default bar cycles as it always did.
        let defaults = resonance_types::default_nav_tabs();
        assert_eq!(
            switch_shown("초보자", &defaults),
            Tab::switch_from("초보자")
        );
        assert_eq!(switch_shown("전체", &defaults), Tab::Custom);
    }

    #[test]
    fn opening_a_tab_clears_its_unread_counts() {
        let counts = || {
            ["WORLD", "GUILD", "PARTY", "SYSTEM"]
                .iter()
                .map(|k| (k.to_string(), 3))
                .collect::<HashMap<_, _>>()
        };
        let custom = vec!["GUILD".to_string(), "PARTY".to_string()];

        let mut c = counts();
        Tab::All.clear_unread(&mut c, &custom);
        assert!(c.is_empty());

        let mut c = counts();
        Tab::Custom.clear_unread(&mut c, &custom);
        assert_eq!(c.len(), 2);
        assert!(c.contains_key("WORLD") && c.contains_key("SYSTEM"));

        let mut c = counts();
        Tab::Channel(Channel::World).clear_unread(&mut c, &custom);
        assert!(!c.contains_key("WORLD") && c.len() == 3);

        let mut c = counts();
        Tab::System.clear_unread(&mut c, &custom);
        assert!(!c.contains_key("SYSTEM") && c.len() == 3);
    }

    #[test]
    fn tab_membership() {
        let custom = vec!["GUILD".to_string()];
        assert!(Tab::All.shows_channel(Channel::Party, &custom));
        assert!(!Tab::System.shows_channel(Channel::Party, &custom));
        assert!(Tab::Custom.shows_channel(Channel::Guild, &custom));
        assert!(!Tab::Custom.shows_channel(Channel::World, &custom));
        assert!(Tab::Channel(Channel::Party).shows_channel(Channel::Party, &custom));
        assert!(!Tab::Channel(Channel::Party).shows_channel(Channel::Guild, &custom));
    }

    #[test]
    fn level_filter_applies_to_world_chat_only() {
        let f = ChatFilter::new(Tab::All, &[], 10, "");
        assert!(!f.matches(&msg(Channel::World, 5, "a", "hi")));
        assert!(f.matches(&msg(Channel::World, 10, "a", "hi")));
        assert!(f.matches(&msg(Channel::Party, 1, "a", "hi")));
    }

    #[test]
    fn search_is_case_insensitive_on_nickname_or_message() {
        let f = ChatFilter::new(Tab::All, &[], 0, "BoB");
        assert!(f.matches(&msg(Channel::World, 1, "bobby", "x")));
        assert!(f.matches(&msg(Channel::World, 1, "x", "hello BOB")));
        assert!(!f.matches(&msg(Channel::World, 1, "alice", "hello")));
    }

    #[test]
    fn system_tab_shows_no_chat() {
        let f = ChatFilter::new(Tab::System, &[], 0, "");
        assert!(!f.matches(&msg(Channel::World, 1, "a", "b")));
    }

    use std::collections::BTreeMap;

    #[test]
    fn scrolling_to_the_top_loads_one_more_page() {
        assert_eq!(next_display_limit(50, 10, false), 100);
        assert_eq!(next_display_limit(150, 49, false), 200);
        assert_eq!(
            next_display_limit(150, LOAD_MORE_ZONE, false),
            150,
            "50 px is not the top"
        );
    }

    #[test]
    fn scrolling_in_the_middle_keeps_the_limit() {
        assert_eq!(next_display_limit(150, 300, false), 150);
    }

    #[test]
    fn coming_back_to_the_bottom_drops_the_older_pages_again() {
        // P-1: the limit used to grow for as long as the overlay was open.
        assert_eq!(next_display_limit(300, 4000, true), DISPLAY_PAGE);
        assert_eq!(next_display_limit(DISPLAY_PAGE, 4000, true), DISPLAY_PAGE);
        // A list shorter than the window is at the bottom and near the top at once:
        // the bottom wins, there is nothing older to load.
        assert_eq!(next_display_limit(300, 0, true), DISPLAY_PAGE);
    }

    #[test]
    fn newest_matching_returns_the_last_page_in_order() {
        let log: BTreeMap<u64, u64> = (1..=10).map(|pid| (pid, pid)).collect();
        assert_eq!(
            newest_matching(log.values().copied(), 3, |_| true),
            [8, 9, 10]
        );
        assert_eq!(
            newest_matching(log.values().copied(), 2, |v| v % 2 == 1),
            [7, 9]
        );
        assert_eq!(
            newest_matching(log.values().copied(), 50, |v| *v > 8),
            [9, 10]
        );
    }

    #[test]
    fn newest_matching_stops_early() {
        let log: BTreeMap<u64, u64> = (1..=1000).map(|pid| (pid, pid)).collect();
        let mut looked_at = 0;
        newest_matching(log.values().copied(), 5, |_| {
            looked_at += 1;
            true
        });
        assert_eq!(looked_at, 5);
    }

    fn limits(pairs: &[(&str, usize)]) -> HashMap<String, usize> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn tab_limits_and_their_defaults() {
        let l = limits(&[("PARTY", 50), ("LOCAL", 30), ("전체", 999), ("커스텀", 999)]);
        let custom = vec!["PARTY".to_string(), "WORLD".to_string()];
        assert_eq!(tab_limit(&l, "PARTY", &custom), 50);
        assert_eq!(tab_limit(&l, "WORLD", &custom), 500); // unset: menu default
        assert_eq!(tab_limit(&l, "GUILD", &custom), 1000);
        // All-tab: every set channel limit together; its own entry is ignored.
        assert_eq!(tab_limit(&l, ALL_TAB, &custom), 80);
        assert_eq!(tab_limit(&HashMap::new(), ALL_TAB, &custom), 2000);
        // Custom: its selected channels together.
        assert_eq!(tab_limit(&l, CUSTOM_TAB, &custom), 550);
        assert_eq!(tab_limit(&l, CUSTOM_TAB, &[]), 1); // never 0: keeps the newest
    }

    #[test]
    fn views_file_each_message_under_its_tabs() {
        let mut v = TabViews::default();
        let custom = vec!["GUILD".to_string()];
        let l = HashMap::new();
        v.add(1, Channel::World, &custom, &l);
        v.add(2, Channel::Guild, &custom, &l);
        assert_eq!(v.pids(ALL_TAB).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(v.pids("WORLD").collect::<Vec<_>>(), [1]);
        assert_eq!(v.pids("GUILD").collect::<Vec<_>>(), [2]);
        assert_eq!(v.pids(CUSTOM_TAB).collect::<Vec<_>>(), [2]);
        assert_eq!(v.pids("PARTY").count(), 0);
    }

    #[test]
    fn busy_world_chat_does_not_evict_guild_messages() {
        let mut v = TabViews::default();
        let l = limits(&[("WORLD", 3), ("GUILD", 2)]); // all-tab: 5
        v.add(1, Channel::Guild, &[], &l);
        let mut dropped = Vec::new();
        for pid in 2..=20 {
            dropped.extend(v.add(pid, Channel::World, &[], &l));
        }
        assert_eq!(v.pids("WORLD").collect::<Vec<_>>(), [18, 19, 20]);
        assert_eq!(v.len(ALL_TAB), 5);
        // The guild message left the all-tab but its own tab still holds it.
        assert_eq!(v.pids("GUILD").collect::<Vec<_>>(), [1]);
        assert!(!dropped.contains(&1));
        // World messages no tab holds are reported, each once.
        assert_eq!(dropped, (2..=15).collect::<Vec<_>>());
    }

    #[test]
    fn a_pid_is_dropped_only_when_its_last_view_lets_go() {
        let mut v = TabViews::default();
        let l = limits(&[("GUILD", 1), ("PARTY", 1)]); // all-tab: 2
        assert!(v.add(1, Channel::Guild, &[], &l).is_empty());
        assert!(v.add(2, Channel::Party, &[], &l).is_empty());
        // 3 pushes 1 out of the all-tab (limit 2) and out of GUILD (limit 1):
        // now no view holds 1.
        let dropped = v.add(3, Channel::Guild, &[], &l);
        assert_eq!(dropped, [1]);
        v.clear();
        assert_eq!(v.len(ALL_TAB), 0);
    }

    #[test]
    fn chat_store_keeps_a_message_while_a_tab_lists_it() {
        let mut store = ChatStore::default();
        let l = limits(&[("WORLD", 2)]); // all-tab: 2
        store.add(1, Channel::Guild, "g1", &[], &l);
        store.add(2, Channel::World, "w2", &[], &l);
        store.add(3, Channel::World, "w3", &[], &l);
        store.add(4, Channel::World, "w4", &[], &l);
        // w2 left both WORLD and the all-tab; g1 left the all-tab but GUILD keeps it.
        assert_eq!(store.get(2), None);
        assert_eq!(store.get(1), Some(&"g1"));
        assert_eq!(store.len(), 3);
        assert_eq!(store.tab(ALL_TAB).collect::<Vec<_>>(), ["w3", "w4"]);
        assert_eq!(store.tab("GUILD").collect::<Vec<_>>(), ["g1"]);
        assert_eq!(store.all(), ["g1", "w3", "w4"]);
        store.clear();
        assert_eq!(store.len(), 0);
        assert_eq!(store.tab(ALL_TAB).count(), 0);
    }

    #[test]
    fn blocked_and_low_level_world_senders_are_muted() {
        let mut m = msg(Channel::World, 5, "a", "hi");
        assert!(!is_muted(&m, 5));
        assert!(is_muted(&m, 6));
        m.channel = Channel::Guild;
        assert!(!is_muted(&m, 6)); // the level rule is WORLD only
        m.is_blocked = true;
        assert!(is_muted(&m, 0));
    }

    type Row = (Channel, &'static str);
    fn row(channel: Channel, tag: &'static str) -> Row {
        (channel, tag)
    }
    fn channel_of(r: &Row) -> Channel {
        r.0
    }

    #[test]
    fn history_merges_under_messages_that_arrived_while_it_loaded() {
        // Regression (A3): hydration replaced the store with the history,
        // dropping messages that arrived (live) during the fetch.
        let (filters, limits) = (vec![], HashMap::new());
        let mut store = ChatStore::default();
        store.add(
            7,
            Channel::World,
            row(Channel::World, "live"),
            &filters,
            &limits,
        );
        let history = vec![
            (3, Channel::Party, row(Channel::Party, "old-a")),
            (5, Channel::World, row(Channel::World, "old-b")),
        ];
        store.merge_history(history, channel_of, &filters, &limits);
        let tags: Vec<_> = store.tab(ALL_TAB).map(|r| r.1).collect();
        assert_eq!(tags, ["old-a", "old-b", "live"]);
        assert_eq!(store.tab("WORLD").count(), 2);
    }

    #[test]
    fn a_message_both_live_and_in_history_keeps_the_live_copy() {
        let (filters, limits) = (vec![], HashMap::new());
        let mut store = ChatStore::default();
        store.add(
            5,
            Channel::World,
            row(Channel::World, "live"),
            &filters,
            &limits,
        );
        let history = vec![(5, Channel::World, row(Channel::World, "snapshot"))];
        store.merge_history(history, channel_of, &filters, &limits);
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(5).unwrap().1, "live");
    }

    #[test]
    fn merged_history_still_obeys_the_tab_limits() {
        let filters = vec![];
        let limits = limits(&[("WORLD", 2)]);
        let mut store = ChatStore::default();
        store.add(
            9,
            Channel::World,
            row(Channel::World, "live"),
            &filters,
            &limits,
        );
        let history = (1..=4)
            .map(|pid| (pid, Channel::World, row(Channel::World, "old")))
            .collect();
        store.merge_history(history, channel_of, &filters, &limits);
        let pids: Vec<_> = store.views.pids("WORLD").collect();
        assert_eq!(pids, [4, 9]);
    }

    #[test]
    fn a_translation_that_beat_its_history_row_is_handed_back_after_the_merge() {
        // K13: a translation event for a row the history fetch had not delivered
        // yet found no row and was lost.
        let (filters, limits) = (vec![], HashMap::new());
        let mut store = ChatStore::default();
        store.hold_translation(5, "번역".to_string());
        store.hold_translation(99, "never arrives".to_string());
        let history = vec![(5, Channel::World, row(Channel::World, "old"))];
        store.merge_history(history, channel_of, &filters, &limits);
        assert_eq!(store.take_held_translations(), [(5, "번역".to_string())]);
        assert!(store.take_held_translations().is_empty());
    }

    #[test]
    fn a_translation_for_a_row_already_held_is_not_kept_aside() {
        let (filters, limits) = (vec![], HashMap::new());
        let mut store = ChatStore::default();
        store.add(
            5,
            Channel::World,
            row(Channel::World, "live"),
            &filters,
            &limits,
        );
        store.hold_translation(5, "x".to_string());
        assert!(store.take_held_translations().is_empty());
    }

    #[test]
    fn translations_held_aside_are_bounded() {
        let mut store = ChatStore::<Row>::default();
        for pid in 1..=(HELD_TRANSLATIONS_MAX as u64 + 50) {
            store.hold_translation(pid, "t".to_string());
        }
        assert_eq!(store.held.len(), HELD_TRANSLATIONS_MAX);
        assert!(!store.held.contains_key(&1), "the oldest goes first");
    }

    /// Counts drops of the value a row signal holds.
    struct Tracked(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn an_evicted_row_signal_is_freed() {
        // Regression (A6): rows were `RwSignal::new` in event callbacks, with
        // no reactive owner, so an evicted row's value was never freed.
        use leptos::prelude::ArcRwSignal;
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (filters, limits) = (vec![], limits(&[("WORLD", 1)]));
        let mut store = ChatStore::default();
        for pid in 1..=3 {
            let row = ArcRwSignal::new(Tracked(dropped.clone()));
            store.add(pid, Channel::World, row, &filters, &limits);
        }
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
