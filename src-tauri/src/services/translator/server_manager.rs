use super::core::{server_url, ServerSlot, PREFERRED_SERVER_PORT};
use crate::protocol::types::{AppState, SystemLogLevel, TranslatorState};
use crate::{inject_system_message, AI_SERVER_FILENAME, AI_SERVER_FOLDER};
use resonance_core::workers::{log_tail, pick_local_port, SERVER_START_TIMEOUT};
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn pid_file(app: &AppHandle) -> Option<PathBuf> {
    let dir = crate::app_dirs::data(app)
        .ok()?
        .join("bin")
        .join(AI_SERVER_FOLDER);
    Some(dir.join("llama-server.pid"))
}

/// Kills `pid` only if it is still a llama-server (a PID can be reused).
fn kill_server_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args([
            "/F",
            "/PID",
            &pid.to_string(),
            "/FI",
            "IMAGENAME eq llama-server.exe",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

pub struct ServerGuard {
    /// `None`: a stand-in server a test run named (`--llama-url`) -- not ours to start, watch or kill.
    child: Option<Child>,
    /// Where this app records the PID of the server it started; `None` with `child`.
    slot: Option<Arc<ServerSlot>>,
    pid_file: Option<PathBuf>,
    log_file: Option<PathBuf>,
}

impl ServerGuard {
    /// The guard of the server `--llama-url` names: nothing started, nothing to kill, never "exited".
    fn external() -> Self {
        Self {
            child: None,
            slot: None,
            pid_file: None,
            log_file: None,
        }
    }

    fn new(app: &AppHandle, child: Child, log_file: Option<PathBuf>) -> Self {
        let pid = child.id();
        let slot = app.state::<AppState>().services.server().clone();
        slot.set_pid(pid);
        let pid_file = pid_file(app);
        if let Some(path) = &pid_file {
            let _ = fs::write(path, pid.to_string());
        }
        Self {
            child: Some(child),
            slot: Some(slot),
            pid_file,
            log_file,
        }
    }

    /// The last lines llama-server wrote (its log file), or "" without one.
    pub fn last_words(&self) -> String {
        self.log_file
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|log| log_tail(&log, 3))
            .unwrap_or_default()
    }

    /// How the server ended, or `None` while it still runs.
    pub fn exit_status(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(e) => Some(format!("state unknown: {e}")),
        }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let pid = child.id();
        let _ = child.kill();
        // A newer server may already have replaced these; only clear our own.
        if let Some(slot) = &self.slot {
            slot.clear_pid(pid);
        }
        if let Some(path) = &self.pid_file {
            if fs::read_to_string(path).is_ok_and(|s| s.trim() == pid.to_string()) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

pub fn launch_ai_server(
    app: &AppHandle,
    model_path: &PathBuf,
    config: &crate::config::AppConfig,
) -> Option<ServerGuard> {
    if let Some(url) = crate::test_env::llama_url() {
        inject_system_message(
            app,
            SystemLogLevel::Info,
            "Translator",
            format!(
                "Using the stand-in server at {url} (--llama-url); no llama-server is started."
            ),
        );
        return Some(ServerGuard::external());
    }
    let Ok(data_dir) = crate::app_dirs::data(app) else {
        let msg = "Failed to start llama-server.exe. (no app data folder)";
        inject_system_message(app, SystemLogLevel::Error, "Translator", msg);
        super::emit_translator_state(app, TranslatorState::Error, msg);
        return None;
    };
    let server_path = data_dir
        .join("bin")
        .join(AI_SERVER_FOLDER)
        .join(AI_SERVER_FILENAME);

    // The server runs elevated from a folder the user can write to: only the files of the
    // pinned zip may be started (review W-2).
    let server_dir = server_path.parent().unwrap_or(&data_dir);
    let problems = resonance_core::server_pins::verify_dir(
        server_dir,
        resonance_core::server_pins::AI_SERVER_PINS,
    );
    if !problems.is_empty() {
        let msg = format!(
            "The AI engine was not started: its files are not the ones this version expects ({}). \
             Delete the folder {} and restart the app to download it again.",
            resonance_core::server_pins::summary(&problems),
            server_dir.display()
        );
        log::error!("[Translator] {msg}");
        inject_system_message(app, SystemLogLevel::Error, "Translator", &msg);
        super::emit_translator_state(app, TranslatorState::Error, &msg);
        return None;
    }

    let mut server_cmd = Command::new(&server_path);
    server_cmd.arg("-m").arg(model_path);
    let port = pick_local_port(PREFERRED_SERVER_PORT);
    app.state::<AppState>().services.server().set_port(port);
    server_cmd.arg("--port").arg(port.to_string());

    // The server's own output goes to a log file (new each start), so an
    // exit can say why -- out of memory, a broken model. Without the file,
    // it logs nothing, as before.
    let log_path = server_path.with_file_name("llama-server.log");
    let log_file = fs::File::create(&log_path)
        .and_then(|out| Ok((out.try_clone()?, out)))
        .ok();
    let log_path = match log_file {
        Some((stdout, stderr)) => {
            server_cmd.stdout(stdout).stderr(stderr);
            Some(log_path)
        }
        None => {
            server_cmd.arg("--log-disable");
            None
        }
    };

    let gpu_layers =
        resonance_core::workers::gpu_layers(config.compute_mode, config.tier).to_string();

    server_cmd.args([
        "-ngl",
        &gpu_layers,
        "-c",
        "1536",
        "-b",
        "64",
        "-ub",
        "64",
        "-t",
        "4",
        "--parallel",
        "1",
    ]);
    server_cmd.creation_flags(CREATE_NO_WINDOW);

    match server_cmd.spawn() {
        Ok(child) => Some(ServerGuard::new(app, child, log_path)),
        Err(e) => {
            let err_msg = format!("Failed to start llama-server.exe. ({})", e);
            inject_system_message(app, SystemLogLevel::Error, "Translator", &err_msg);
            super::emit_translator_state(
                app,
                TranslatorState::Error,
                &format!("Failed to start llama-server.exe. ({})", e),
            );
            None
        }
    }
}

/// Waits until the server answers `/health`. Gives up when the process
/// exits (at once, with its exit status), after `SERVER_START_TIMEOUT`, or
/// when `keep_waiting` turns false.
pub fn wait_for_server(
    app: &AppHandle,
    server: &mut ServerGuard,
    keep_waiting: &dyn Fn() -> bool,
) -> Result<(), String> {
    let client = resonance_llama::client();
    let start_wait = Instant::now();

    inject_system_message(
        app,
        SystemLogLevel::Info,
        "Translator",
        "Waiting for AI Engine to warm up...",
    );

    while start_wait.elapsed() < SERVER_START_TIMEOUT {
        if !keep_waiting() {
            return Err("AI Engine start cancelled.".into());
        }
        if let Some(status) = server.exit_status() {
            let last_words = server.last_words();
            return Err(if last_words.is_empty() {
                format!(
                    "AI Engine failed to start: llama-server exited ({status}). Out of memory or a broken model?"
                )
            } else {
                format!("AI Engine failed to start: llama-server exited ({status}): {last_words}")
            });
        }
        // Once a second for as long as the model loads: the log file only, not
        // the 200-line system history, where it pushes out the useful lines.
        log::trace!("[Translator] Polling {}/health...", server_url(app));

        if resonance_llama::health_ok(&client, &server_url(app)) {
            inject_system_message(
                app,
                SystemLogLevel::Trace,
                "Translator",
                format!(
                    "Health check passed after {}ms",
                    start_wait.elapsed().as_millis()
                ),
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(1000));
    }

    Err(format!(
        "AI Engine failed to initialize within {}s.",
        SERVER_START_TIMEOUT.as_secs()
    ))
}

#[tauri::command]
pub async fn ai_server_health_check(app: AppHandle) -> bool {
    // async: a blocking HTTP call here would freeze the window
    match reqwest::get(format!("{}/health", server_url(&app))).await {
        Ok(res) => res.status().is_success(),
        Err(_) => {
            inject_system_message(
                &app,
                SystemLogLevel::Error,
                "Translator",
                "AI Engine is unavailable.",
            );
            false
        }
    }
}

/// Stops the llama-server this app started, and one left behind by a
/// previous run that crashed (recorded in the PID file).
pub fn kill_orphaned_servers(app: &AppHandle) {
    inject_system_message(
        app,
        SystemLogLevel::Info,
        "Translator",
        "Cleaning up any orphaned AI server processes...",
    );

    let current = app.state::<AppState>().services.server().take_pid();
    if current != 0 {
        kill_server_pid(current);
    }
    if let Some(path) = pid_file(app) {
        if let Some(pid) = fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
        {
            if pid != current {
                kill_server_pid(pid);
            }
        }
        let _ = fs::remove_file(path);
    }
}
