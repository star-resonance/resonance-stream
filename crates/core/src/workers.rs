//! Decisions about the app's background workers, kept free of Tauri so they
//! can be tested on any OS.

use resonance_types::{ComputeMode, Tier};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// What to do with a worker after a settings change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerChange {
    Keep,
    Start,
    Stop,
    Restart,
}

/// The settings that decide whether and how the translator runs.
#[derive(Debug, Clone, Copy)]
pub struct TranslatorSettings {
    pub enabled: bool,
    pub compute_mode: ComputeMode,
    pub tier: Tier,
}

/// How many model layers llama-server puts on the GPU (`-ngl`): none on the
/// CPU, by VRAM tier on the GPU.
pub fn gpu_layers(compute_mode: ComputeMode, tier: Tier) -> u32 {
    match (compute_mode, tier) {
        (ComputeMode::Cpu, _) => 0,
        (ComputeMode::Gpu, Tier::Low) => 12,
        (ComputeMode::Gpu, Tier::Middle) => 24,
        (ComputeMode::Gpu, Tier::High) => 32,
        (ComputeMode::Gpu, Tier::VeryHigh) => 99,
    }
}

/// Exactly one change for the translator, given old and new settings.
pub fn translator_change(old: TranslatorSettings, new: TranslatorSettings) -> WorkerChange {
    match (old.enabled, new.enabled) {
        (false, true) => WorkerChange::Start,
        (true, false) => WorkerChange::Stop,
        (true, true) if old.compute_mode != new.compute_mode || old.tier != new.tier => {
            WorkerChange::Restart
        }
        _ => WorkerChange::Keep,
    }
}

/// What a settings change does to the sniffer, given the adapter it was bound to and the one it
/// should be bound to now (`""` is automatic). A changed adapter drops the running capture; a
/// new one starts only once the first-run setup is done, so the wizard's own saves never open a
/// socket. `Start` is never answered: the sniffer is started by the ui, not by a setting.
pub fn sniffer_change(old_interface: &str, new_interface: &str, init_done: bool) -> WorkerChange {
    match (old_interface != new_interface, init_done) {
        (false, _) => WorkerChange::Keep,
        (true, true) => WorkerChange::Restart,
        (true, false) => WorkerChange::Stop,
    }
}

/// How often the sniffer's watchdog looks at the clock.
pub const WATCHDOG_TICK: Duration = Duration::from_secs(5);

/// Seconds without a game packet after which the watchdog says so.
pub const WATCHDOG_LIMIT_SECS: u64 = 15;

/// What the watchdog found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogVerdict {
    /// The capture thread is gone (it failed while setting up, or ended): that has its own
    /// message, and "no game traffic" would only replace it. The watchdog stops.
    CaptureEnded,
    /// No packet has been recorded yet (`last_traffic` is 0): nothing to judge.
    NotStarted,
    Healthy,
    /// More than [`WATCHDOG_LIMIT_SECS`] since the last packet: report it and look again later.
    Stalled,
}

/// The watchdog's decision, from whether the capture thread still runs and the Unix seconds of the
/// last game packet and of now (passed in, so it is tested without a clock). A clock that stepped
/// back is not a stall.
pub fn watchdog_check(last_traffic: u64, now: u64, capture_alive: bool) -> WatchdogVerdict {
    if !capture_alive {
        WatchdogVerdict::CaptureEnded
    } else if last_traffic == 0 {
        WatchdogVerdict::NotStarted
    } else if now.saturating_sub(last_traffic) > WATCHDOG_LIMIT_SECS {
        WatchdogVerdict::Stalled
    } else {
        WatchdogVerdict::Healthy
    }
}

/// How long a restart waits for the old capture thread to end before it goes on anyway. A read
/// times out after half a second, so a healthy thread ends well inside this.
pub const SNIFFER_STOP_WAIT: Duration = Duration::from_secs(3);

/// Polls `condition` every `poll` until it holds or `timeout` has passed, and says whether it
/// held. It is looked at once more when the time is up, so a thread that ends right at the
/// deadline is not reported as stuck. This replaces "sleep a while and hope" (review W-10).
pub fn wait_for(mut condition: impl FnMut() -> bool, timeout: Duration, poll: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return condition();
        }
        std::thread::sleep(poll.min(deadline - now));
    }
}

/// A translation that waited longer than this is skipped: by then its chat
/// row has scrolled away, and translating it only delays the newer ones.
pub const MAX_TRANSLATION_WAIT: Duration = Duration::from_secs(60);

/// A port for the local translation server: `preferred` when it is free,
/// otherwise one the OS picks. 8080 is a common development port, so the
/// server must not assume it.
pub fn pick_local_port(preferred: u16) -> u16 {
    let bind = |port: u16| std::net::TcpListener::bind(("127.0.0.1", port));
    bind(preferred)
        .or_else(|_| bind(0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .unwrap_or(preferred)
}

/// How long the sniffer waits before reading again after `failures_in_a_row`
/// failed reads (1 = the first): doubling from 10 ms up to 1 s, so a socket
/// that keeps failing does not spin a core.
pub fn read_error_backoff(failures_in_a_row: u32) -> Duration {
    const FIRST: Duration = Duration::from_millis(10);
    const MAX: Duration = Duration::from_secs(1);
    let doublings = failures_in_a_row.saturating_sub(1).min(16);
    FIRST.saturating_mul(1 << doublings).min(MAX)
}

pub fn translation_is_stale(queued_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(queued_at) > MAX_TRANSLATION_WAIT
}

/// How long a starting llama-server may take to answer `/health`. Loading
/// a model from a cold disk can take a while; a server that dies is noticed
/// at once (the wait checks the process), so a generous limit costs nothing.
pub const SERVER_START_TIMEOUT: Duration = Duration::from_secs(90);

/// First wait before restarting a failed server; doubled for each restart
/// within `RESTART_WINDOW`.
pub const RESTART_BACKOFF: Duration = Duration::from_secs(2);
/// Restarts allowed within `RESTART_WINDOW` before giving up.
pub const MAX_RESTARTS: usize = 3;
pub const RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Failed jobs in a row, with the process still alive, that mark the server
/// as hung.
pub const HUNG_AFTER_FAILURES: u32 = 3;

/// What the translator worker does after a job or a server check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorAction {
    Continue,
    /// Kill the server, wait this long, start a new one.
    Restart(Duration),
    /// Too many restarts: leave translation stopped until the user restarts it.
    GiveUp,
}

/// Decides when the translator's llama-server is restarted: when its process
/// exited, or when it stopped answering while alive (hung). Restarts back
/// off, and stop after `MAX_RESTARTS` within `RESTART_WINDOW`.
#[derive(Debug, Default)]
pub struct ServerSupervisor {
    restarts: Vec<Instant>,
    failures_in_a_row: u32,
}

impl ServerSupervisor {
    pub fn on_job_ok(&mut self) {
        self.failures_in_a_row = 0;
    }

    /// A job failed while the server process is still running.
    pub fn on_job_failed(&mut self, now: Instant) -> SupervisorAction {
        self.failures_in_a_row += 1;
        if self.failures_in_a_row < HUNG_AFTER_FAILURES {
            return SupervisorAction::Continue;
        }
        self.restart(now)
    }

    /// The server process is gone.
    pub fn on_server_exited(&mut self, now: Instant) -> SupervisorAction {
        self.restart(now)
    }

    fn restart(&mut self, now: Instant) -> SupervisorAction {
        self.failures_in_a_row = 0;
        self.restarts
            .retain(|at| now.saturating_duration_since(*at) <= RESTART_WINDOW);
        if self.restarts.len() >= MAX_RESTARTS {
            return SupervisorAction::GiveUp;
        }
        let backoff = RESTART_BACKOFF * 2u32.pow(self.restarts.len() as u32);
        self.restarts.push(now);
        SupervisorAction::Restart(backoff)
    }
}

/// Longest log line `log_tail` keeps (then `…`).
pub const LOG_TAIL_LINE_CHARS: usize = 200;

/// The last `n` non-empty lines of a server log, joined by " | ", for an
/// error message: why llama-server exited is usually its last word.
pub fn log_tail(log: &str, n: usize) -> String {
    let mut lines: Vec<String> = log
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(n)
        .map(|line| {
            if line.chars().count() > LOG_TAIL_LINE_CHARS {
                let cut: String = line.chars().take(LOG_TAIL_LINE_CHARS).collect();
                cut + "…"
            } else {
                line.to_string()
            }
        })
        .collect();
    lines.reverse();
    lines.join(" | ")
}

/// Most pids the ledger holds; the oldest go first. Only the newest few are
/// ever caught up, so this only bounds memory while translation stays off.
pub const LEDGER_CAPACITY: usize = 5000;

/// The translation ledger: the Japanese chat messages of this run (it starts
/// empty when the app opens) that are still owed a translation. A message
/// leaves it only when its translation succeeds -- or when a catch-up passes
/// it over as too old -- so one that failed, went stale in the queue or
/// arrived while the translator was off is still owed at the next start.
#[derive(Debug, Default)]
pub struct TranslationLedger {
    owed: BTreeSet<u64>,
}

impl TranslationLedger {
    /// A new Japanese message, owed until translated.
    pub fn record(&mut self, pid: u64) {
        self.owed.insert(pid);
        while self.owed.len() > LEDGER_CAPACITY {
            self.owed.pop_first();
        }
    }

    /// Translated (or gone): no longer owed. Returns whether it was owed.
    pub fn settle(&mut self, pid: u64) -> bool {
        self.owed.remove(&pid)
    }

    pub fn is_owed(&self, pid: u64) -> bool {
        self.owed.contains(&pid)
    }

    pub fn len(&self) -> usize {
        self.owed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.owed.is_empty()
    }

    /// At a translator start: the newest `limit` owed pids, oldest first.
    /// They stay owed until settled; the older ones are passed over and
    /// dropped (0 catches up nothing and drops everything).
    pub fn catch_up(&mut self, limit: usize) -> Vec<u64> {
        let keep_from = self.owed.len().saturating_sub(limit);
        let newest = match self.owed.iter().nth(keep_from).copied() {
            Some(first_kept) => self.owed.split_off(&first_kept),
            None => BTreeSet::new(),
        };
        self.owed = newest;
        self.owed.iter().copied().collect()
    }
}

/// Which translator worker is current. A worker that was stopped or replaced while it was still
/// starting up sees the number move and stays silent instead of reporting on a server that is not
/// its own. Owned by `Services`, one per app run.
#[derive(Debug, Default)]
pub struct WorkerGenerations(AtomicU64);

impl WorkerGenerations {
    /// A new worker starts: it becomes the current one, and every earlier one is superseded.
    pub fn begin(&self) -> u64 {
        0
    }

    /// Marks every running or starting worker as superseded (the translator was stopped).
    pub fn retire(&self) {}

    /// Is the worker that began as `generation` still the current one?
    pub fn is_current(&self, _generation: u64) -> bool {
        false
    }
}

/// Whether game traffic is coming in: the Unix seconds of the last game packet (0: none yet) and
/// whether the sniffer has said "active" since it last said otherwise. Owned by `Services` for the
/// whole app run, not by one sniffer start, so a restart keeps the time of the last packet and the
/// watchdog still speaks when a new adapter sees nothing.
#[derive(Debug, Default)]
pub struct SnifferActivity {
    last_traffic: AtomicU64,
    active: AtomicBool,
}

impl SnifferActivity {
    /// A game packet arrived at `now`. Returns `true` when the sniffer was not active, so the caller
    /// emits the "active" state once, not for every packet.
    pub fn record_packet(&self, _now: u64) -> bool {
        false
    }

    /// A sniffer starts at `now`: the watchdog gets a full [`WATCHDOG_LIMIT_SECS`] before it speaks,
    /// even when the adapter never sees a game packet. Whether it is active is left alone.
    pub fn start_clock(&self, _now: u64) {}

    /// The sniffer said something other than "active" (starting, pending): the next packet says
    /// "active" again.
    pub fn mark_inactive(&self) {}

    /// The watchdog reported a stall at `now`: not active, and the clock starts over, so it waits
    /// another [`WATCHDOG_LIMIT_SECS`] before it speaks again.
    pub fn stall(&self, _now: u64) {}

    /// Unix seconds of the last game packet, 0 before the first.
    pub fn last_traffic(&self) -> u64 {
        u64::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ComputeMode::{Cpu, Gpu};
    use Tier::{High, Low, Middle, VeryHigh};

    fn s(enabled: bool, compute_mode: ComputeMode, tier: Tier) -> TranslatorSettings {
        TranslatorSettings {
            enabled,
            compute_mode,
            tier,
        }
    }

    #[test]
    fn the_gpu_takes_layers_by_tier_and_the_cpu_none() {
        // The numbers llama-server's `-ngl` has always been given.
        assert_eq!(
            [Low, Middle, High, VeryHigh].map(|tier| gpu_layers(Gpu, tier)),
            [12, 24, 32, 99]
        );
        for tier in Tier::ALL {
            assert_eq!(gpu_layers(Cpu, *tier), 0, "{tier:?}");
        }
    }

    #[test]
    fn turning_translation_on_starts_exactly_one_worker() {
        // Regression (review B4): save_config started two.
        assert_eq!(
            translator_change(s(false, Cpu, Middle), s(true, Cpu, Middle)),
            WorkerChange::Start
        );
    }

    // --- the sniffer: what a settings change does, and when the watchdog speaks ---------------

    #[test]
    fn the_sniffer_keeps_running_while_its_adapter_is_unchanged() {
        for init_done in [false, true] {
            assert_eq!(sniffer_change("", "", init_done), WorkerChange::Keep);
            assert_eq!(
                sniffer_change("Ethernet", "Ethernet", init_done),
                WorkerChange::Keep
            );
        }
    }

    #[test]
    fn a_changed_adapter_restarts_the_sniffer_once_setup_is_done() {
        // auto -> a named adapter, a named one -> another, a named one -> auto.
        for (old, new) in [("", "Ethernet"), ("Ethernet", "Wi-Fi"), ("Wi-Fi", "")] {
            assert_eq!(
                sniffer_change(old, new, true),
                WorkerChange::Restart,
                "{old:?} -> {new:?}"
            );
        }
    }

    #[test]
    fn a_changed_adapter_only_stops_the_sniffer_before_setup_is_done() {
        // The old capture is dropped; nothing is started until the wizard has finished.
        assert_eq!(sniffer_change("", "Ethernet", false), WorkerChange::Stop);
    }

    #[test]
    fn the_watchdog_is_quiet_until_traffic_has_been_recorded() {
        assert_eq!(
            watchdog_check(0, 1_000_000, true),
            WatchdogVerdict::NotStarted
        );
        assert_eq!(watchdog_check(0, 0, true), WatchdogVerdict::NotStarted);
    }

    #[test]
    fn the_watchdog_speaks_after_more_than_fifteen_seconds_of_silence() {
        let last = 1_000_000;
        assert_eq!(watchdog_check(last, last, true), WatchdogVerdict::Healthy);
        assert_eq!(
            watchdog_check(last, last + 15, true),
            WatchdogVerdict::Healthy,
            "exactly the limit"
        );
        assert_eq!(
            watchdog_check(last, last + 16, true),
            WatchdogVerdict::Stalled
        );
        assert_eq!(
            watchdog_check(last, last + 3600, true),
            WatchdogVerdict::Stalled
        );
    }

    #[test]
    fn a_clock_that_steps_back_does_not_make_the_watchdog_speak() {
        assert_eq!(
            watchdog_check(1_000_000, 999_000, true),
            WatchdogVerdict::Healthy
        );
    }

    #[test]
    fn the_watchdog_looks_every_five_seconds_for_a_fifteen_second_limit() {
        assert_eq!(WATCHDOG_TICK, Duration::from_secs(5));
        assert_eq!(WATCHDOG_LIMIT_SECS, 15);
    }

    #[test]
    fn a_watchdog_over_a_capture_that_has_ended_stops_instead_of_blaming_the_traffic() {
        // The capture thread died while setting up (not admin, no adapter ...): that was reported
        // with its own error, and "no game traffic" 15 s later would only replace it.
        assert_eq!(
            watchdog_check(0, 1_000_000, false),
            WatchdogVerdict::CaptureEnded
        );
        assert_eq!(
            watchdog_check(1_000_000, 1_000_100, false),
            WatchdogVerdict::CaptureEnded
        );
        assert_eq!(
            watchdog_check(1_000_000, 1_000_001, false),
            WatchdogVerdict::CaptureEnded
        );
    }

    // --- waiting for a thread to end instead of sleeping and hoping (review W-10) --------------

    #[test]
    fn a_condition_that_already_holds_returns_at_once() {
        let started = Instant::now();
        assert!(wait_for(
            || true,
            Duration::from_secs(5),
            Duration::from_millis(10)
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_condition_another_thread_makes_true_is_waited_for() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let flag = Arc::new(AtomicBool::new(false));
        let setter = {
            let flag = flag.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(60));
                flag.store(true, Ordering::SeqCst);
            })
        };
        let got = wait_for(
            || flag.load(Ordering::SeqCst),
            Duration::from_secs(5),
            Duration::from_millis(5),
        );
        setter.join().unwrap();
        assert!(got);
    }

    #[test]
    fn a_condition_that_never_holds_gives_up_after_the_timeout() {
        let started = Instant::now();
        assert!(!wait_for(
            || false,
            Duration::from_millis(80),
            Duration::from_millis(10)
        ));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(80),
            "waited only {waited:?}"
        );
        assert!(waited < Duration::from_secs(2), "waited {waited:?}");
    }

    #[test]
    fn the_condition_is_looked_at_once_more_when_the_time_is_up() {
        // A thread that ends right at the deadline must not be reported as stuck.
        let mut calls = 0;
        let got = wait_for(
            || {
                calls += 1;
                calls >= 2
            },
            Duration::ZERO,
            Duration::from_millis(1),
        );
        assert!(got, "looked {calls} time(s)");
    }

    #[test]
    fn the_old_capture_gets_three_seconds_to_end() {
        // A read times out after 0.5 s, so a healthy thread ends well inside this.
        assert_eq!(SNIFFER_STOP_WAIT, Duration::from_secs(3));
    }

    #[test]
    fn translator_transitions() {
        use WorkerChange::*;
        let on = s(true, Gpu, High);
        assert_eq!(translator_change(on, s(false, Gpu, High)), Stop);
        assert_eq!(translator_change(on, on), Keep);
        assert_eq!(translator_change(on, s(true, Gpu, Low)), Restart);
        assert_eq!(translator_change(on, s(true, Cpu, High)), Restart);
        // Spec changes while off do not start anything.
        assert_eq!(
            translator_change(s(false, Cpu, Low), s(false, Gpu, High)),
            Keep
        );
    }

    #[test]
    fn picks_the_preferred_port_when_free() {
        // Ask the OS for a free port, release it, then prefer it.
        let free = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert_eq!(pick_local_port(free), free);
    }

    #[test]
    fn avoids_a_busy_preferred_port() {
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        let picked = pick_local_port(busy_port);
        assert_ne!(picked, busy_port);
        assert_ne!(picked, 0);
    }

    #[test]
    fn staleness() {
        let t0 = Instant::now();
        assert!(!translation_is_stale(t0, t0));
        assert!(!translation_is_stale(t0, t0 + MAX_TRANSLATION_WAIT));
        assert!(translation_is_stale(
            t0,
            t0 + MAX_TRANSLATION_WAIT + Duration::from_millis(1)
        ));
    }

    #[test]
    fn ledger_owes_a_message_until_it_is_translated() {
        let mut ledger = TranslationLedger::default();
        assert!(ledger.is_empty());
        ledger.record(5);
        ledger.record(7);
        assert!(ledger.is_owed(5));
        assert!(ledger.settle(5));
        assert!(!ledger.settle(5)); // a second translation of it is not owed
        assert!(!ledger.is_owed(5));
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn a_failed_message_is_still_owed_after_later_ones_succeed() {
        let mut ledger = TranslationLedger::default();
        for pid in [1, 2, 3] {
            ledger.record(pid);
        }
        ledger.settle(3); // 2 failed, 3 succeeded
        ledger.settle(1);
        assert_eq!(ledger.catch_up(10), [2]);
    }

    #[test]
    fn catch_up_takes_the_newest_and_drops_the_rest() {
        let mut ledger = TranslationLedger::default();
        for pid in [10, 11, 12, 13, 14] {
            ledger.record(pid);
        }
        assert_eq!(ledger.catch_up(2), [13, 14]);
        assert_eq!(ledger.len(), 2); // still owed until translated
        assert!(!ledger.is_owed(12)); // passed over for good
        assert_eq!(ledger.catch_up(5), [13, 14]);
        assert!(ledger.catch_up(0).is_empty());
        assert!(ledger.is_empty());
    }

    #[test]
    fn ledger_keeps_only_the_newest_up_to_its_capacity() {
        let mut ledger = TranslationLedger::default();
        for pid in 0..(LEDGER_CAPACITY as u64 + 3) {
            ledger.record(pid);
        }
        assert_eq!(ledger.len(), LEDGER_CAPACITY);
        assert!(!ledger.is_owed(2));
        assert!(ledger.is_owed(3));
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_server_that_exited_is_restarted_with_growing_backoff() {
        // Regression (A2): a crashed llama-server was never noticed; every
        // later job failed silently while the badge said "Active".
        let mut sup = ServerSupervisor::default();
        let now = t0();
        assert_eq!(
            sup.on_server_exited(now),
            SupervisorAction::Restart(RESTART_BACKOFF)
        );
        assert_eq!(
            sup.on_server_exited(now + Duration::from_secs(10)),
            SupervisorAction::Restart(RESTART_BACKOFF * 2)
        );
    }

    #[test]
    fn a_server_that_keeps_crashing_is_given_up_on() {
        let mut sup = ServerSupervisor::default();
        let now = t0();
        for i in 0..MAX_RESTARTS {
            let at = now + Duration::from_secs(i as u64);
            assert!(matches!(
                sup.on_server_exited(at),
                SupervisorAction::Restart(_)
            ));
        }
        assert_eq!(
            sup.on_server_exited(now + Duration::from_secs(60)),
            SupervisorAction::GiveUp
        );
    }

    #[test]
    fn crashes_long_ago_do_not_count() {
        let mut sup = ServerSupervisor::default();
        let now = t0();
        for _ in 0..MAX_RESTARTS {
            sup.on_server_exited(now);
        }
        let later = now + RESTART_WINDOW + Duration::from_secs(1);
        assert_eq!(
            sup.on_server_exited(later),
            SupervisorAction::Restart(RESTART_BACKOFF)
        );
    }

    #[test]
    fn a_live_server_is_restarted_only_after_several_failures_in_a_row() {
        let mut sup = ServerSupervisor::default();
        let now = t0();
        for _ in 1..HUNG_AFTER_FAILURES {
            assert_eq!(sup.on_job_failed(now), SupervisorAction::Continue);
        }
        sup.on_job_ok();
        for _ in 1..HUNG_AFTER_FAILURES {
            assert_eq!(sup.on_job_failed(now), SupervisorAction::Continue);
        }
        assert_eq!(
            sup.on_job_failed(now),
            SupervisorAction::Restart(RESTART_BACKOFF)
        );
        // ...and the count starts over for the new server.
        assert_eq!(sup.on_job_failed(now), SupervisorAction::Continue);
    }

    #[test]
    fn the_last_lines_of_a_server_log_explain_its_exit() {
        let log = "load model\n\nggml_vulkan: out of memory\r\nfailed to load model\n\n";
        assert_eq!(
            log_tail(log, 2),
            "ggml_vulkan: out of memory | failed to load model"
        );
        assert_eq!(log_tail("", 3), "");
        assert_eq!(log_tail("one", 3), "one");
    }

    #[test]
    fn a_failing_socket_read_backs_off_up_to_a_second() {
        let ms = |n| read_error_backoff(n).as_millis();
        assert_eq!((ms(1), ms(2), ms(3), ms(4)), (10, 20, 40, 80));
        assert_eq!(ms(8), 1000); // 10 * 2^7 = 1280, capped
        assert_eq!(ms(1_000_000), 1000); // no overflow
        assert!(ms(0) <= 10); // not a failure yet: no wait to speak of
    }

    #[test]
    fn a_long_log_line_is_cut() {
        let tail = log_tail(&"x".repeat(1000), 1);
        assert!(tail.chars().count() <= LOG_TAIL_LINE_CHARS + 1);
    }

    #[test]
    fn a_worker_that_began_is_current() {
        let generations = WorkerGenerations::default();
        let first = generations.begin();
        assert!(generations.is_current(first));
    }

    #[test]
    fn a_new_worker_supersedes_the_one_before() {
        let generations = WorkerGenerations::default();
        let first = generations.begin();
        let second = generations.begin();
        assert_ne!(first, second);
        assert!(!generations.is_current(first));
        assert!(generations.is_current(second));
    }

    #[test]
    fn a_retired_translator_leaves_no_worker_current() {
        let generations = WorkerGenerations::default();
        let worker = generations.begin();
        generations.retire();
        assert!(!generations.is_current(worker));
    }

    #[test]
    fn no_traffic_is_recorded_before_the_first_packet() {
        let activity = SnifferActivity::default();
        assert_eq!(activity.last_traffic(), 0);
        // ...so the watchdog has nothing to judge yet.
        assert_eq!(
            watchdog_check(activity.last_traffic(), 1_000, true),
            WatchdogVerdict::NotStarted
        );
    }

    #[test]
    fn a_started_sniffer_that_sees_no_packet_is_reported_after_the_limit() {
        let activity = SnifferActivity::default();
        activity.start_clock(1_000);
        assert_eq!(
            watchdog_check(activity.last_traffic(), 1_000 + WATCHDOG_LIMIT_SECS, true),
            WatchdogVerdict::Healthy
        );
        assert_eq!(
            watchdog_check(
                activity.last_traffic(),
                1_000 + WATCHDOG_LIMIT_SECS + 1,
                true
            ),
            WatchdogVerdict::Stalled
        );
        // Starting the clock is not a packet: the first one still says "active".
        assert!(activity.record_packet(1_001));
    }

    #[test]
    fn only_the_first_packet_says_the_sniffer_became_active() {
        let activity = SnifferActivity::default();
        assert!(activity.record_packet(1_000));
        assert!(!activity.record_packet(1_001));
        assert_eq!(activity.last_traffic(), 1_001);
    }

    #[test]
    fn after_another_state_the_next_packet_says_active_again() {
        let activity = SnifferActivity::default();
        activity.record_packet(1_000);
        activity.mark_inactive();
        assert!(activity.record_packet(1_002));
    }

    #[test]
    fn a_stall_starts_the_watchdog_clock_over_and_waits_for_active_again() {
        let activity = SnifferActivity::default();
        activity.record_packet(1_000);
        let now = 1_000 + WATCHDOG_LIMIT_SECS + 1;
        assert_eq!(
            watchdog_check(activity.last_traffic(), now, true),
            WatchdogVerdict::Stalled
        );
        activity.stall(now);
        assert_eq!(activity.last_traffic(), now);
        assert_eq!(
            watchdog_check(activity.last_traffic(), now + 1, true),
            WatchdogVerdict::Healthy
        );
        assert!(activity.record_packet(now + 2));
    }
}
