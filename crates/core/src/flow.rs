//! What happens to a chat line between the capture and the translation: the sniffer's dispatch of
//! the pipeline's actions ([`dispatch`]) and the translator's job ([`run_translation_job`]). The
//! app and the test harness (`crates/llama/tests/support/harness.rs`) both run this code; each
//! gives it a [`Sink`] for the effects (history, events, archive, translator queue).

use crate::capture::PipelineAction;
use crate::history::{ArchiveTarget, ChatHistory};
use crate::text::{
    contains_japanese, convert_to_romaji, nicknames_in, preprocess_with_nicknames,
    translate_masked, Dictionary, TranslationCache,
};
use crate::workers::TranslationLedger;
use resonance_types::ChatMessage;
use std::collections::HashMap;
use std::sync::Arc;

/// Where the flow's effects go. The `with_*` methods hand out shared state for as long as the
/// closure runs, so an app that keeps it behind a lock holds the lock only that long.
pub trait Sink {
    /// Is translation switched on in the settings. Read once per batch of actions.
    fn use_translation(&mut self) -> bool;
    /// Stores `chat` in the backend's history and shows it (`packet-event`).
    fn store_and_emit(&mut self, chat: &ChatMessage);
    /// Tells the windows a stored row changed (`chat-message-update`).
    fn emit_update(&mut self, chat: &ChatMessage);
    /// Tells the windows a row has its translation (`translation-event`).
    fn emit_translation(&mut self, pid: u64, translated: &str);
    /// Queues `chat` for the archive files `target` names.
    fn archive(&mut self, chat: &ChatMessage, target: ArchiveTarget);
    /// Hands `chat` to the translator worker; `false` when none is running.
    fn queue_translation(&mut self, chat: ChatMessage) -> bool;
    /// The dictionary in use.
    fn dictionary(&mut self) -> Arc<Dictionary>;
    fn with_history<R>(&mut self, f: impl FnOnce(&mut ChatHistory) -> R) -> R;
    fn with_ledger<R>(&mut self, f: impl FnOnce(&mut TranslationLedger) -> R) -> R;
    /// Japanese nickname -> romaji, filled as senders arrive.
    fn with_nicknames<R>(&mut self, f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R;
}

/// The sniffer's side: what becomes of each action the capture pipeline produced.
pub fn dispatch<S: Sink>(sink: &mut S, actions: Vec<PipelineAction>) {
    let use_translation = sink.use_translation();

    for action in actions {
        match action {
            PipelineAction::UpdateBlockedMessage(chat) => {
                let flagged = sink.with_history(|history| {
                    let row = history.get_mut(chat.pid)?;
                    if row.is_blocked {
                        return None;
                    }
                    row.is_blocked = true;
                    Some(row.clone())
                });
                if let Some(row) = flagged {
                    sink.emit_update(&row);
                }
            }
            PipelineAction::EmitNewMessage(mut chat) => {
                // Apply Romaji Swap
                if contains_japanese(&chat.nickname) {
                    let nickname = chat.nickname.clone();
                    let romaji = sink.with_nicknames(|cache| {
                        cache
                            .entry(nickname.clone())
                            .or_insert_with(|| convert_to_romaji(&nickname))
                            .clone()
                    });
                    chat.nickname_romaji = Some(romaji);
                }

                // Dispatch Side Effects. (A duplicate never gets here: the pipeline
                // dropped it, so it is neither translated nor archived.)
                sink.store_and_emit(&chat);

                // Every Japanese message is owed a translation, even with the
                // translator off or still starting: its next start catches up.
                let japanese = contains_japanese(&chat.message);
                if japanese {
                    sink.with_ledger(|ledger| ledger.record(chat.pid));
                }

                // Every message is in the chat log from the moment it arrives; what
                // the translator does later only adds to the training-pair file and a
                // newer chat-log line with the translation.
                sink.archive(&chat, ArchiveTarget::ChatLog);
                let queued = use_translation && japanese && sink.queue_translation(chat.clone());
                if !queued {
                    // No translation will come: the training-pair file gets it as it is.
                    sink.archive(&chat, ArchiveTarget::Dataset);
                }
            }
        }
    }
}

/// How a translation job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobResult {
    Done,
    /// Nothing was owed (translated already, or not of this run).
    Skipped,
    /// No translation; the message stays owed.
    Failed,
}

/// The translator's side: one queued line. `translate` sends the shielded text to the server
/// (only for a line the cache does not have).
pub fn run_translation_job<S: Sink>(
    sink: &mut S,
    chat: &ChatMessage,
    cache: &mut TranslationCache,
    translate: impl FnOnce(&str) -> Result<String, String>,
) -> JobResult {
    // Translated already (a catch-up item that was also queued live) or not
    // a message of this run's ledger: nothing is owed.
    if !sink.with_ledger(|ledger| ledger.is_owed(chat.pid)) {
        return JobResult::Skipped;
    }

    // 1. Preprocess. The nicknames are only picked under `with_nicknames`: the sniffer needs the
    // cache for every Japanese nickname, and must wait neither for the rest of the shielding nor
    // for the HTTP round trip below.
    let dict = sink.dictionary();
    let nicknames = sink.with_nicknames(|cache| nicknames_in(&chat.message, cache));
    let shield = preprocess_with_nicknames(&chat.message, &dict, &nicknames);

    // 2. HTTP request, unless this line was translated before; 3. postprocess.
    let final_str = match translate_masked(&shield, cache, translate) {
        Ok(text) => text,
        Err(reason) => {
            // No translation: the row stays as it is, and the original is
            // still archived.
            log::warn!("[Translator] pid {}: {}", chat.pid, reason);
            sink.archive(chat, ArchiveTarget::Dataset);
            return JobResult::Failed;
        }
    };

    // 4. Dispatch Side Effects: the translated copy goes to both archives (the chat log
    // already holds the message from its arrival; `load_recent` keeps the newest line).
    let mut archived = chat.clone();
    archived.translated = Some(final_str.clone());
    sink.archive(&archived, ArchiveTarget::Both);

    sink.with_history(|history| {
        if let Some(row) = history.get_mut(chat.pid) {
            row.translated = Some(final_str.clone());
        }
    });
    sink.with_ledger(|ledger| {
        ledger.settle(chat.pid);
    });
    sink.emit_translation(chat.pid, &final_str);
    JobResult::Done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::PipelineAction;
    use crate::history::{ArchiveTarget, ChannelLimits, ChatHistory};
    use crate::text::{Dictionary, TranslationCache};
    use crate::workers::TranslationLedger;
    use resonance_types::{Channel, ChatMessage};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A sink that keeps everything in fields and writes down every effect, in order.
    struct Recorder {
        use_translation: bool,
        translator_running: bool,
        history: ChatHistory,
        ledger: TranslationLedger,
        nicknames: HashMap<String, String>,
        dictionary: Arc<Dictionary>,
        effects: Vec<String>,
    }

    impl Recorder {
        fn new() -> Self {
            Self {
                use_translation: true,
                translator_running: true,
                history: ChatHistory::new(ChannelLimits::new(&HashMap::new())),
                ledger: TranslationLedger::default(),
                nicknames: HashMap::new(),
                dictionary: Arc::new(Dictionary::default()),
                effects: Vec::new(),
            }
        }
    }

    impl Sink for Recorder {
        fn use_translation(&mut self) -> bool {
            self.use_translation
        }
        fn store_and_emit(&mut self, chat: &ChatMessage) {
            self.history.push(chat.clone());
            self.effects.push(format!("store {}", chat.pid));
        }
        fn emit_update(&mut self, chat: &ChatMessage) {
            self.effects
                .push(format!("update {} blocked={}", chat.pid, chat.is_blocked));
        }
        fn emit_translation(&mut self, pid: u64, translated: &str) {
            self.effects.push(format!("translation {pid} {translated}"));
        }
        fn archive(&mut self, chat: &ChatMessage, target: ArchiveTarget) {
            let text = chat.translated.as_deref().unwrap_or("-");
            self.effects
                .push(format!("archive {} {target:?} {text}", chat.pid));
        }
        fn queue_translation(&mut self, chat: ChatMessage) -> bool {
            if self.translator_running {
                self.effects.push(format!("queue {}", chat.pid));
            }
            self.translator_running
        }
        fn dictionary(&mut self) -> Arc<Dictionary> {
            self.dictionary.clone()
        }
        fn with_history<R>(&mut self, f: impl FnOnce(&mut ChatHistory) -> R) -> R {
            f(&mut self.history)
        }
        fn with_ledger<R>(&mut self, f: impl FnOnce(&mut TranslationLedger) -> R) -> R {
            f(&mut self.ledger)
        }
        fn with_nicknames<R>(&mut self, f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
            f(&mut self.nicknames)
        }
    }

    fn line(pid: u64, nickname: &str, message: &str) -> ChatMessage {
        ChatMessage {
            pid,
            channel: Channel::World,
            nickname: nickname.into(),
            message: message.into(),
            ..Default::default()
        }
    }

    fn new_line(pid: u64, message: &str) -> PipelineAction {
        PipelineAction::EmitNewMessage(line(pid, "Bob", message))
    }

    #[test]
    fn a_japanese_line_is_stored_owed_logged_on_arrival_then_queued() {
        let mut sink = Recorder::new();
        dispatch(&mut sink, vec![new_line(1, "こんにちは")]);
        assert_eq!(sink.effects, ["store 1", "archive 1 ChatLog -", "queue 1"]);
        assert!(sink.ledger.is_owed(1));
    }

    #[test]
    fn with_translation_off_or_no_translator_the_line_goes_to_the_dataset_as_it_is() {
        for (use_translation, translator_running) in [(false, true), (true, false)] {
            let mut sink = Recorder::new();
            sink.use_translation = use_translation;
            sink.translator_running = translator_running;
            dispatch(&mut sink, vec![new_line(1, "こんにちは")]);
            assert_eq!(
                sink.effects,
                ["store 1", "archive 1 ChatLog -", "archive 1 Dataset -"],
                "use_translation {use_translation}, translator {translator_running}"
            );
            // Still owed: the translator's next start catches it up.
            assert!(sink.ledger.is_owed(1));
        }
    }

    #[test]
    fn a_line_with_no_japanese_is_never_owed_nor_queued() {
        let mut sink = Recorder::new();
        dispatch(&mut sink, vec![new_line(1, "gg wp")]);
        assert_eq!(
            sink.effects,
            ["store 1", "archive 1 ChatLog -", "archive 1 Dataset -"]
        );
        assert!(!sink.ledger.is_owed(1));
    }

    #[test]
    fn a_japanese_nickname_gets_its_romaji_once_and_it_is_kept() {
        let mut sink = Recorder::new();
        dispatch(
            &mut sink,
            vec![
                PipelineAction::EmitNewMessage(line(1, "たろう", "gg")),
                PipelineAction::EmitNewMessage(line(2, "Bob", "gg")),
            ],
        );
        let romaji = sink.history.get(1).unwrap().nickname_romaji.clone();
        assert!(romaji.as_deref().is_some_and(|r| !r.is_empty()));
        assert_eq!(sink.nicknames.get("たろう"), romaji.as_ref());
        assert_eq!(sink.history.get(2).unwrap().nickname_romaji, None);
        // A cached romaji is used as it is.
        sink.nicknames.insert("たろう".into(), "Taro".into());
        dispatch(
            &mut sink,
            vec![PipelineAction::EmitNewMessage(line(3, "たろう", "gg"))],
        );
        assert_eq!(
            sink.history.get(3).unwrap().nickname_romaji.as_deref(),
            Some("Taro")
        );
    }

    #[test]
    fn a_blocked_update_flags_a_stored_row_once_and_ignores_an_unknown_one() {
        let mut sink = Recorder::new();
        dispatch(&mut sink, vec![new_line(1, "gg")]);
        sink.effects.clear();
        let update = || PipelineAction::UpdateBlockedMessage(line(1, "Bob", "gg"));
        dispatch(&mut sink, vec![update(), update()]);
        dispatch(
            &mut sink,
            vec![PipelineAction::UpdateBlockedMessage(line(9, "Bob", "gg"))],
        );
        assert_eq!(sink.effects, ["update 1 blocked=true"]);
        assert!(sink.history.get(1).unwrap().is_blocked);
    }

    /// A line that arrived and is owed, as `dispatch` leaves it.
    fn owed(sink: &mut Recorder, pid: u64, message: &str) -> ChatMessage {
        dispatch(sink, vec![new_line(pid, message)]);
        sink.effects.clear();
        sink.history.get(pid).unwrap().clone()
    }

    #[test]
    fn a_translated_job_archives_both_sets_the_row_settles_and_emits() {
        let mut sink = Recorder::new();
        let chat = owed(&mut sink, 1, "こんにちは");
        let mut cache = TranslationCache::new(8);
        let result = run_translation_job(&mut sink, &chat, &mut cache, |masked| {
            assert_eq!(masked, "こんにちは");
            Ok("안녕하세요".into())
        });
        assert_eq!(result, JobResult::Done);
        assert_eq!(
            sink.effects,
            ["archive 1 Both 안녕하세요", "translation 1 안녕하세요"]
        );
        assert_eq!(
            sink.history.get(1).unwrap().translated.as_deref(),
            Some("안녕하세요")
        );
        assert!(!sink.ledger.is_owed(1));
    }

    #[test]
    fn a_failed_job_archives_the_line_as_it_is_and_it_stays_owed() {
        let mut sink = Recorder::new();
        let chat = owed(&mut sink, 1, "こんにちは");
        let mut cache = TranslationCache::new(8);
        let result = run_translation_job(&mut sink, &chat, &mut cache, |_| Err("timeout".into()));
        assert_eq!(result, JobResult::Failed);
        assert_eq!(sink.effects, ["archive 1 Dataset -"]);
        assert_eq!(sink.history.get(1).unwrap().translated, None);
        assert!(sink.ledger.is_owed(1));
    }

    #[test]
    fn a_job_nothing_is_owed_for_does_nothing() {
        let mut sink = Recorder::new();
        let chat = line(7, "Bob", "こんにちは");
        let mut cache = TranslationCache::new(8);
        let result = run_translation_job(&mut sink, &chat, &mut cache, |_| {
            panic!("no server call for a line that is not owed")
        });
        assert_eq!(result, JobResult::Skipped);
        assert!(sink.effects.is_empty());
    }

    #[test]
    fn a_repeated_line_is_served_from_the_cache_without_a_server_call() {
        let mut sink = Recorder::new();
        let mut cache = TranslationCache::new(8);
        let first = owed(&mut sink, 1, "もう一回");
        run_translation_job(&mut sink, &first, &mut cache, |_| Ok("한 번 더".into()));
        let again = owed(&mut sink, 2, "もう一回");
        let result = run_translation_job(&mut sink, &again, &mut cache, |_| {
            panic!("the cache has this line")
        });
        assert_eq!(result, JobResult::Done);
        assert_eq!(
            sink.history.get(2).unwrap().translated.as_deref(),
            Some("한 번 더")
        );
    }

    #[test]
    fn a_job_shields_the_known_nicknames_in_the_line() {
        let mut sink = Recorder::new();
        sink.nicknames.insert("たろう".into(), "Taro".into());
        let chat = owed(&mut sink, 1, "たろうさん、こんにちは");
        let mut cache = TranslationCache::new(8);
        let mut sent = String::new();
        run_translation_job(&mut sink, &chat, &mut cache, |masked| {
            sent = masked.to_string();
            Ok(masked.to_string())
        });
        assert!(
            !sent.contains("たろう"),
            "the nickname reached the server: {sent}"
        );
        assert!(sink
            .history
            .get(1)
            .unwrap()
            .translated
            .as_deref()
            .is_some_and(|t| t.contains("Taro")));
    }
}
