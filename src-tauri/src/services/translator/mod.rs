pub mod core;
pub mod server_manager;

pub use server_manager::*;

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use reqwest::blocking::Client;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::inject_system_message;
use crate::protocol::types::{ChatMessage, SystemLogLevel, TranslatorState};

use self::core::server_url;
use resonance_core::history::ArchiveTarget;
use resonance_core::text::{
    nicknames_in, preprocess_with_nicknames, translate_masked, TranslationCache,
    TRANSLATION_CACHE_SIZE,
};
use resonance_core::workers::{
    translation_is_stale, ServerSupervisor, SupervisorAction, WorkerGenerations,
};
use resonance_llama::translate_text;

pub struct TranslationJob {
    pub chat: ChatMessage,
    pub queued_at: Instant,
}

impl TranslationJob {
    pub fn new(chat: ChatMessage) -> Self {
        Self {
            chat,
            queued_at: Instant::now(),
        }
    }
}

/// Starts a translator worker; `generations` (owned by `Services`) tells it whether it is still
/// the current one.
pub fn start_translator_worker(
    app: AppHandle,
    model_path: PathBuf,
    generations: Arc<WorkerGenerations>,
) -> Sender<TranslationJob> {
    let (tx, rx) = unbounded::<TranslationJob>();
    if crate::test_env::no_translator() {
        // As if the translator were off: no server, and each chat is archived
        // as it is.
        emit_translator_state(
            &app,
            TranslatorState::Off,
            "Translator disabled (--no-translator)",
        );
        thread::spawn(move || {
            for job in rx {
                archive_untranslated(&app, &job.chat);
            }
        });
        return tx;
    }
    let generation = generations.begin();
    let is_current = move || generations.is_current(generation);
    let config = crate::config::current_config(&app);

    thread::spawn(move || {
        inject_system_message(
            &app,
            SystemLogLevel::Info,
            "Translator",
            "Initializing HTTP AI Backend...",
        );
        emit_translator_state(
            &app,
            TranslatorState::Starting,
            "Initializing AI Backend...",
        );

        // The dictionary lives in AppState, so a sync or an edit applies to
        // the next job without a restart.
        let client = resonance_llama::client();
        let mut supervisor = ServerSupervisor::default();
        // Outlives server restarts: the same model answers the same way.
        let mut cache = TranslationCache::new(TRANSLATION_CACHE_SIZE);

        // One pass per llama-server: a server that crashed or hung is
        // replaced (with backoff) until the supervisor gives up.
        loop {
            if !is_current() {
                return;
            }
            server_manager::kill_orphaned_servers(&app);

            // 1. Launch the Server
            let Some(mut server) = server_manager::launch_ai_server(&app, &model_path, &config)
            else {
                return;
            };

            // 2. Wait for Health
            emit_translator_state(
                &app,
                TranslatorState::LoadingModel,
                "Loading AI weights into VRAM...",
            );
            let started = server_manager::wait_for_server(&app, &mut server, &is_current);
            if !is_current() {
                return; // superseded while loading: the guard drops and kills our server
            }
            if let Err(reason) = started {
                // A server that died while loading -- the port was taken
                // between picking it and binding it, say -- is restarted like
                // one that died later: a fresh port, a backoff, a limit.
                if server.exit_status().is_some() {
                    if let SupervisorAction::Restart(wait) =
                        supervisor.on_server_exited(Instant::now())
                    {
                        let msg = format!("{reason} Retrying in {}s...", wait.as_secs());
                        inject_system_message(&app, SystemLogLevel::Warning, "Translator", &msg);
                        emit_translator_state(&app, TranslatorState::Restarting, &msg);
                        drop(server);
                        if !sleep_while(wait, &is_current) {
                            return;
                        }
                        continue;
                    }
                }
                inject_system_message(&app, SystemLogLevel::Error, "Translator", &reason);
                emit_translator_state(&app, TranslatorState::Error, &reason);
                drain_untranslated(&app, &rx);
                return;
            }

            inject_system_message(
                &app,
                SystemLogLevel::Success,
                "Translator",
                "AI Server running! Ready for translation.",
            );

            let mut run = ServerRun {
                app: &app,
                client: &client,
                server: &mut server,
                supervisor: &mut supervisor,
                cache: &mut cache,
            };

            // 3. Once per server: what the ledger says was missed
            let mut outcome = catch_up(&mut run, &rx, &is_current);
            if outcome == Some(SupervisorAction::Continue) {
                if !is_current() {
                    return;
                }
                emit_translator_state(&app, TranslatorState::Active, "AI Engine Ready");
                // 4. The translation loop
                outcome = serve(&mut run, &rx);
            }

            match outcome {
                None | Some(SupervisorAction::Continue) => return, // stopped
                Some(SupervisorAction::Restart(wait)) => {
                    let msg = format!("AI Engine stopped. Restarting in {}s...", wait.as_secs());
                    inject_system_message(&app, SystemLogLevel::Warning, "Translator", &msg);
                    emit_translator_state(&app, TranslatorState::Restarting, &msg);
                    drop(server); // kills a hung server before the wait
                    if !sleep_while(wait, &is_current) {
                        return;
                    }
                }
                Some(SupervisorAction::GiveUp) => {
                    let msg = "AI Engine keeps stopping. Turn translation off and on to retry.";
                    inject_system_message(&app, SystemLogLevel::Error, "Translator", msg);
                    emit_translator_state(&app, TranslatorState::Error, msg);
                    drop(server);
                    drain_untranslated(&app, &rx);
                    return;
                }
            }
        }
    });

    tx
}

/// How often an idle worker checks that its server is still running.
const IDLE_CHECK: Duration = Duration::from_secs(5);

/// One running llama-server and what decides its fate.
struct ServerRun<'a> {
    app: &'a AppHandle,
    client: &'a Client,
    server: &'a mut server_manager::ServerGuard,
    supervisor: &'a mut ServerSupervisor,
    cache: &'a mut TranslationCache,
}

impl ServerRun<'_> {
    /// Translates `job` and says what to do next: a failed job whose
    /// server exited, or the last of several failures in a row, restarts it.
    fn translate(&mut self, job: TranslationJob) -> SupervisorAction {
        match process_translation_job(job, self.client, self.app, self.cache) {
            JobResult::Done => {
                self.supervisor.on_job_ok();
                SupervisorAction::Continue
            }
            JobResult::Skipped => SupervisorAction::Continue,
            JobResult::Failed => match self.server.exit_status() {
                Some(status) => self.server_exited(&status),
                None => self.supervisor.on_job_failed(Instant::now()),
            },
        }
    }

    fn live(&mut self, job: TranslationJob) -> SupervisorAction {
        if translation_is_stale(job.queued_at, Instant::now()) {
            log::debug!("[Translator] Skipped pid {}: waited too long", job.chat.pid);
            archive_untranslated(self.app, &job.chat);
            return SupervisorAction::Continue;
        }
        self.translate(job)
    }

    fn server_exited(&mut self, status: &str) -> SupervisorAction {
        let last_words = self.server.last_words();
        let msg = if last_words.is_empty() {
            format!("llama-server exited ({}).", status)
        } else {
            format!("llama-server exited ({}): {}", status, last_words)
        };
        inject_system_message(self.app, SystemLogLevel::Warning, "Translator", msg);
        self.supervisor.on_server_exited(Instant::now())
    }
}

/// Translates live jobs until the channel closes (`None`: the translator
/// was stopped) or the server must be restarted or given up on.
fn serve(run: &mut ServerRun, rx: &Receiver<TranslationJob>) -> Option<SupervisorAction> {
    loop {
        let action = match rx.recv_timeout(IDLE_CHECK) {
            Ok(job) => run.live(job),
            Err(RecvTimeoutError::Timeout) => match run.server.exit_status() {
                Some(status) => run.server_exited(&status),
                None => SupervisorAction::Continue,
            },
            Err(RecvTimeoutError::Disconnected) => return None,
        };
        if action != SupervisorAction::Continue {
            return Some(action);
        }
    }
}

/// Sleeps `total`, in short steps; `false` if the worker was superseded.
fn sleep_while(total: Duration, is_current: &dyn Fn() -> bool) -> bool {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if !is_current() {
            return false;
        }
        thread::sleep(Duration::from_millis(200));
    }
    is_current()
}

/// No server any more: archive what still arrives, untranslated (it stays
/// owed in the ledger), until the translator is stopped or replaced.
fn drain_untranslated(app: &AppHandle, rx: &Receiver<TranslationJob>) {
    while let Ok(job) = rx.recv() {
        archive_untranslated(app, &job.chat);
    }
}

/// Translates, once per server start, the Japanese messages of this run
/// that the ledger still owes: the newest `translation_catch_up_limit` of
/// them, oldest first, read from the in-memory history. Jobs queued while
/// the server loaded are dropped first -- their messages are in the ledger.
/// Live messages arriving meanwhile go before the next catch-up item.
/// `None`: superseded; otherwise `Continue`, or the restart decision that
/// cut it short.
fn catch_up(
    run: &mut ServerRun,
    rx: &Receiver<TranslationJob>,
    is_current: &dyn Fn() -> bool,
) -> Option<SupervisorAction> {
    let app = run.app;
    let state = app.state::<crate::AppState>();
    while rx.try_recv().is_ok() {}

    let limit = crate::config::current_config(app).translation_catch_up_limit;
    let pids = state.translation_ledger.lock().catch_up(limit);
    let owed: Vec<ChatMessage> = {
        let history = state.chat_history.lock();
        let mut ledger = state.translation_ledger.lock();
        pids.into_iter()
            .filter_map(|pid| match history.get(pid) {
                Some(chat) if chat.translated.is_none() => Some(chat.clone()),
                _ => {
                    ledger.settle(pid); // gone from history, or translated already
                    None
                }
            })
            .collect()
    };
    if owed.is_empty() {
        return Some(SupervisorAction::Continue);
    }

    let total = owed.len();
    inject_system_message(
        app,
        SystemLogLevel::Info,
        "Translator",
        format!("Catching up {} missed message(s)...", total),
    );
    for (done, chat) in owed.into_iter().enumerate() {
        if !is_current() {
            return None;
        }
        emit_translator_state(
            app,
            TranslatorState::CatchingUp,
            &format!("{}/{}", done + 1, total),
        );
        while let Ok(job) = rx.try_recv() {
            let action = run.live(job);
            if action != SupervisorAction::Continue {
                return Some(action);
            }
        }
        let action = run.translate(TranslationJob::new(chat));
        if action != SupervisorAction::Continue {
            return Some(action);
        }
    }
    let still_owed = state.translation_ledger.lock().len();
    inject_system_message(
        app,
        SystemLogLevel::Info,
        "Translator",
        format!("Catch-up done ({} still untranslated).", still_owed),
    );
    Some(SupervisorAction::Continue)
}

/// Queues `chat` for the archive when its channel is archived.
fn archive(app: &AppHandle, chat: &ChatMessage, target: ArchiveTarget) {
    let state = app.state::<crate::AppState>();
    if !crate::io::archives_channel(app, chat.channel) {
        return;
    }
    let df_tx = state.data_factory_tx.lock().clone();
    if let Some(df_tx) = df_tx {
        let _ = df_tx.send(crate::io::DataFactoryJob {
            chat: chat.clone(),
            target,
        });
    }
}

/// A message has just arrived: it goes to the chat log at once, whatever the translator does
/// or fails to do with it (review W-4). Before the translation job is queued, so a
/// translated copy always comes after it in the log.
pub fn archive_arrival(app: &AppHandle, chat: &ChatMessage) {
    archive(app, chat, ArchiveTarget::ChatLog);
}

/// No translation will come for `chat`: it goes to the training-pair file as it is. The chat
/// log has it since it arrived.
pub fn archive_untranslated(app: &AppHandle, chat: &ChatMessage) {
    archive(app, chat, ArchiveTarget::Dataset);
}

enum JobResult {
    Done,
    /// Nothing was owed (translated already, or not of this run).
    Skipped,
    /// No translation; the message stays owed.
    Failed,
}

fn process_translation_job(
    job: TranslationJob,
    client: &Client,
    app: &AppHandle,
    cache: &mut TranslationCache,
) -> JobResult {
    let chat = job.chat;
    let state = app.state::<crate::AppState>();

    // Translated already (a catch-up item that was also queued live) or not
    // a message of this run's ledger: nothing is owed.
    if !state.translation_ledger.lock().is_owed(chat.pid) {
        return JobResult::Skipped;
    }

    // 1. Preprocess. The nickname lock is held only to pick the names this message
    // contains: the sniffer needs it for every Japanese nickname, and must wait neither
    // for the rest of the shielding nor for the HTTP round trip below.
    let dict = state.dictionary.read().clone();
    let nicknames = nicknames_in(&chat.message, &state.nickname_cache.lock());
    let shield = preprocess_with_nicknames(&chat.message, &dict, &nicknames);

    // 2. HTTP Request (Blocking), unless this line was translated before;
    // 3. Postprocess
    let final_str = match translate_masked(&shield, cache, |masked| {
        translate_text(client, &server_url(app), masked)
    }) {
        Ok(text) => text,
        Err(reason) => {
            // No translation: the row stays as it is, and the original is
            // still archived.
            log::warn!("[Translator] pid {}: {}", chat.pid, reason);
            archive_untranslated(app, &chat);
            return JobResult::Failed;
        }
    };

    // 4. Dispatch Side Effects: the translated copy goes to both archives (the chat log
    // already holds the message from its arrival; `load_recent` keeps the newest line).
    let mut archived = chat.clone();
    archived.translated = Some(final_str.clone());
    archive(app, &archived, ArchiveTarget::Both);

    if let Some(existing_chat) = state.chat_history.lock().get_mut(chat.pid) {
        existing_chat.translated = Some(final_str.clone());
    }
    state.translation_ledger.lock().settle(chat.pid);

    let _ = app.emit(
        "translation-event",
        &crate::protocol::types::TranslationResult {
            pid: chat.pid,
            translated: final_str,
        },
    );
    JobResult::Done
}

/// Records the state (for `get_service_states`) and emits it. The lock is
/// held across the emit so events leave in `seq` order.
pub fn emit_translator_state(app: &tauri::AppHandle, state: TranslatorState, message: &str) {
    let Some(app_state) = app.try_state::<crate::AppState>() else {
        return;
    };
    let mut states = app_state.service_states.lock();
    let payload = states.set_translator(state, message);
    let _ = app.emit("translator-state", payload);
}
