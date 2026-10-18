//! What the app does between the two servers, without Tauri: the same code the app runs
//! (`resonance_core::flow`: the sniffer's dispatch and the translator's job), with a `Sink` that
//! keeps the history, the ledger and the nickname cache in fields instead of the app's state.

use super::llama_server::MockLlama;
use reqwest::blocking::Client;
use resonance_core::capture::ChatPipeline;
use resonance_core::flow::{self, Sink};
use resonance_core::history::{ArchiveTarget, ChannelLimits, ChatHistory};
use resonance_core::text::{Dictionary, TranslationCache, TRANSLATION_CACHE_SIZE};
use resonance_core::workers::TranslationLedger;
use resonance_llama::translate_text;
use resonance_types::ChatMessage;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The capture side: raw packets in, chat rows out.
pub struct Capture {
    pub pipeline: ChatPipeline,
    pub blocked: HashSet<u64>,
    next_pid: u64,
    /// Updated rows for newly blocked users (`chat-message-update`).
    pub blocked_updates: Vec<ChatMessage>,
    /// Japanese nickname -> romaji, as the dispatch fills it.
    pub nicknames: HashMap<String, String>,
    history: ChatHistory,
    ledger: TranslationLedger,
    dictionary: Arc<Dictionary>,
    /// Is a translator running (the `Harness` has one; a `Capture` alone does not).
    translating: bool,
    /// Rows stored by the current `feed`, and lines queued for the translator.
    stored: Vec<ChatMessage>,
    queued: Vec<ChatMessage>,
}

impl Capture {
    pub fn new() -> Self {
        Self {
            pipeline: ChatPipeline::new(),
            blocked: HashSet::new(),
            next_pid: 0,
            blocked_updates: Vec::new(),
            nicknames: HashMap::new(),
            history: ChatHistory::new(ChannelLimits::new(&HashMap::new())),
            ledger: TranslationLedger::default(),
            dictionary: Arc::new(Dictionary::default()),
            translating: false,
            stored: Vec::new(),
            queued: Vec::new(),
        }
    }

    /// Every new row the packets produce, in order.
    pub fn feed(&mut self, packets: &[Vec<u8>]) -> Vec<ChatMessage> {
        for packet in packets {
            let blocked = &self.blocked;
            let next_pid = &mut self.next_pid;
            let actions = self.pipeline.feed_network_packet(
                packet,
                |uid| blocked.contains(&uid),
                || {
                    *next_pid += 1;
                    *next_pid
                },
                || {},
            );
            flow::dispatch(self, actions);
        }
        std::mem::take(&mut self.stored)
    }

    pub fn texts(&mut self, packets: &[Vec<u8>]) -> Vec<String> {
        self.feed(packets).into_iter().map(|c| c.message).collect()
    }
}

impl Sink for Capture {
    fn use_translation(&mut self) -> bool {
        true
    }
    fn store_and_emit(&mut self, chat: &ChatMessage) {
        self.history.push(chat.clone());
        self.stored.push(chat.clone());
    }
    fn emit_update(&mut self, chat: &ChatMessage) {
        self.blocked_updates.push(chat.clone());
    }
    fn emit_translation(&mut self, _pid: u64, _translated: &str) {}
    fn archive(&mut self, _chat: &ChatMessage, _target: ArchiveTarget) {}
    fn queue_translation(&mut self, chat: ChatMessage) -> bool {
        if self.translating {
            self.queued.push(chat);
        }
        self.translating
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

/// The translator side, talking to a llama-server at `url`.
pub struct Translator {
    pub client: Client,
    pub url: String,
    pub dictionary: Dictionary,
    pub cache: TranslationCache,
}

impl Translator {
    pub fn new(llama: &MockLlama, client: Client) -> Self {
        Self {
            client,
            url: llama.url.clone(),
            dictionary: Dictionary::default(),
            cache: TranslationCache::new(TRANSLATION_CACHE_SIZE),
        }
    }
}

/// Both ends wired together: rows with the translation of every line the dispatch queued (the
/// Japanese ones), each run through the translator's job.
pub struct Harness {
    pub capture: Capture,
    pub translator: Translator,
}

impl Harness {
    pub fn new(llama: &MockLlama) -> Self {
        let mut capture = Capture::new();
        capture.translating = true;
        Self {
            capture,
            translator: Translator::new(llama, resonance_llama::client()),
        }
    }

    pub fn run(&mut self, packets: &[Vec<u8>]) -> Vec<ChatMessage> {
        let mut rows = self.capture.feed(packets);
        self.capture.dictionary = Arc::new(self.translator.dictionary.clone());
        for chat in std::mem::take(&mut self.capture.queued) {
            let Translator {
                client, url, cache, ..
            } = &mut self.translator;
            flow::run_translation_job(&mut self.capture, &chat, cache, |masked| {
                translate_text(client, url, masked)
            });
        }
        for row in &mut rows {
            row.translated = self
                .capture
                .history
                .get(row.pid)
                .and_then(|stored| stored.translated.clone());
        }
        rows
    }
}
