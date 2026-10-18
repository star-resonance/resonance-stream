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
use resonance_core::flow::{JobResult, Sink};
use resonance_core::history::{ArchiveTarget, ChatHistory};
use resonance_core::text::{TranslationCache, TRANSLATION_CACHE_SIZE};
use resonance_core::workers::TranslationLedger;
use resonance_core::workers::{
    translation_is_stale, ServerSupervisor, SupervisorAction, WorkerGenerations,
};
use resonance_llama::translate_text;
use std::collections::HashMap;

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

/// No translation will come for `chat`: it goes to the training-pair file as it is. The chat
/// log has it since it arrived.
pub fn archive_untranslated(app: &AppHandle, chat: &ChatMessage) {
    archive(app, chat, ArchiveTarget::Dataset);
}

/// Where `resonance_core::flow` puts its effects in the running app: the `AppState`, the
/// windows, the archive worker and the translator worker.
pub struct AppSink<'a> {
    app: &'a AppHandle,
}

impl<'a> AppSink<'a> {
    pub fn new(app: &'a AppHandle) -> Self {
        Self { app }
    }

    fn state(&self) -> tauri::State<'a, crate::AppState> {
        self.app.state::<crate::AppState>()
    }
}

impl Sink for AppSink<'_> {
    fn use_translation(&mut self) -> bool {
        self.state().config.read().use_translation
    }

    fn store_and_emit(&mut self, chat: &ChatMessage) {
        crate::store_and_emit(self.app, chat.clone());
    }

    fn emit_update(&mut self, chat: &ChatMessage) {
        let _ = self
            .app
            .emit(resonance_types::ipc::event::CHAT_MESSAGE_UPDATE, chat);
    }

    fn emit_translation(&mut self, pid: u64, translated: &str) {
        let _ = self.app.emit(
            resonance_types::ipc::event::TRANSLATION_EVENT,
            &crate::protocol::types::TranslationResult {
                pid,
                translated: translated.to_string(),
            },
        );
    }

    fn archive(&mut self, chat: &ChatMessage, target: ArchiveTarget) {
        archive(self.app, chat, target);
    }

    fn queue_translation(&mut self, chat: ChatMessage) -> bool {
        match self.state().services.translator_sender() {
            Some(tx) => {
                let _ = tx.send(TranslationJob::new(chat));
                true
            }
            None => false,
        }
    }

    fn dictionary(&mut self) -> Arc<resonance_core::text::Dictionary> {
        self.state().dictionary.read().clone()
    }

    fn with_history<R>(&mut self, f: impl FnOnce(&mut ChatHistory) -> R) -> R {
        f(&mut self.state().chat_history.lock())
    }

    fn with_ledger<R>(&mut self, f: impl FnOnce(&mut TranslationLedger) -> R) -> R {
        f(&mut self.state().translation_ledger.lock())
    }

    fn with_nicknames<R>(&mut self, f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
        f(&mut self.state().nickname_cache.lock())
    }
}

/// One queued line: `resonance_core::flow::run_translation_job` with the app's effects and the
/// running llama-server.
fn process_translation_job(
    job: TranslationJob,
    client: &Client,
    app: &AppHandle,
    cache: &mut TranslationCache,
) -> JobResult {
    resonance_core::flow::run_translation_job(&mut AppSink::new(app), &job.chat, cache, |masked| {
        translate_text(client, &server_url(app), masked)
    })
}

/// Records the state (for `get_service_states`) and emits it. The lock is
/// held across the emit so events leave in `seq` order.
pub fn emit_translator_state(app: &tauri::AppHandle, state: TranslatorState, message: &str) {
    let Some(app_state) = app.try_state::<crate::AppState>() else {
        return;
    };
    let mut states = app_state.service_states.lock();
    let payload = states.set_translator(state, message);
    let _ = app.emit(resonance_types::ipc::event::TRANSLATOR_STATE, payload);
}
