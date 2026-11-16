//! The decisions behind patching and unpatching the game's code while its other threads are
//! suspended: where a thread's instruction pointer must not be, how the trampoline is
//! found, which thread is the main one, and how install and drain retry. All pure, so the
//! rules are tested on any OS; `win/threads.rs` and `win/hook.rs` feed them real values.

use crate::slot::Clock;
use std::ops::Range;

/// Threads the DLL will suspend at most (more is refused: `hook_failed:too_many_threads`).
pub const MAX_SUSPEND_THREADS: usize = 1024;
/// Tries to patch or unpatch while a thread is inside the window.
pub const INSTALL_ATTEMPTS: u32 = 5;
/// Pause between two tries, outside the suspension.
pub const RETRY_SLEEP_MS: u64 = 2;
/// ilhook's trampoline is one 1024-byte allocation.
pub const TRAMPOLINE_SPAN: usize = 1024;
/// How long to wait after the last detour has left before the code may go.
pub const DRAIN_GRACE_MS: u64 = 50;
/// How long to wait for a detour that is still running.
pub const DRAIN_MAX_MS: u64 = 2000;
/// The longest detail an install failure carries (the pipe cuts at 256 anyway).
const MAX_DETAIL: usize = 256;

/// Patching is unsafe while a suspended thread's `Rip` is inside the bytes being rewritten.
pub fn patch_blocked(ips: &[usize], window: Range<usize>) -> bool {
    ips.iter().any(|ip| window.contains(ip))
}

/// Unpatching must also keep clear of the trampoline, the DLL's own image and any detour
/// still running (a thread deep in the game's code called by the pump is counted by
/// `in_detour`).
pub fn unhook_blocked(
    ips: &[usize],
    window: Range<usize>,
    tramp: usize,
    dll: Range<usize>,
    in_detour: usize,
) -> bool {
    let tramp_range = tramp..tramp.saturating_add(TRAMPOLINE_SPAN);
    in_detour != 0
        || patch_blocked(ips, window)
        || patch_blocked(ips, tramp_range)
        || patch_blocked(ips, dll)
}

/// Where the 14-byte patch at `at` jumps to: `E9 rel32` or `FF 25 00000000` + an 8-byte
/// address (ilhook writes one of the two).
pub fn decode_jmp_target(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes {
        [0xE9, rel @ ..] if rel.len() >= 4 => {
            let rel = i32::from_le_bytes(rel[..4].try_into().ok()?);
            Some(at.wrapping_add(5).wrapping_add_signed(rel as isize))
        }
        [0xFF, 0x25, 0, 0, 0, 0, addr @ ..] if addr.len() >= 8 => {
            Some(u64::from_le_bytes(addr[..8].try_into().ok()?) as usize)
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooManyThreads;

/// The thread ids to suspend: this process's, minus the worker's own, in order. `entries`
/// are `(owner process id, thread id)` pairs.
pub fn threads_of(
    entries: &[(u32, u32)],
    pid: u32,
    own_tid: u32,
    cap: usize,
) -> Result<Vec<u32>, TooManyThreads> {
    let tids: Vec<u32> = entries
        .iter()
        .filter(|(owner, tid)| *owner == pid && *tid != own_tid)
        .map(|(_, tid)| *tid)
        .collect();
    if tids.len() > cap {
        return Err(TooManyThreads);
    }
    Ok(tids)
}

/// The process's first thread (Unity's main thread, estimated): the earliest creation time.
pub fn pick_main_thread(threads: &[(u32, u64)]) -> Option<u32> {
    threads
        .iter()
        .min_by_key(|(_, created)| *created)
        .map(|(tid, _)| *tid)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// A thread was inside the window on every try: nothing is wrong with the candidate.
    Transient,
    /// This candidate cannot be hooked (the detail says why).
    Unsupported(String),
}

/// Tries each candidate in order; `install` gets its name. A transient failure is retried
/// up to [`INSTALL_ATTEMPTS`] times with [`RETRY_SLEEP_MS`] between (and then returned: the
/// next candidate would meet the same threads); an unsupported one moves on. The result
/// names the candidate that worked.
pub fn install_with_retry<T>(
    candidates: &[&str],
    mut install: impl FnMut(&str) -> Result<T, InstallError>,
    mut sleep_ms: impl FnMut(u64),
) -> Result<(String, T), InstallError> {
    let mut reasons: Vec<String> = Vec::new();
    for name in candidates {
        let mut attempt = 1;
        loop {
            match install(name) {
                Ok(value) => return Ok((name.to_string(), value)),
                Err(InstallError::Unsupported(why)) => {
                    reasons.push(format!("{name}:{why}"));
                    break;
                }
                Err(InstallError::Transient) if attempt >= INSTALL_ATTEMPTS => {
                    return Err(InstallError::Transient)
                }
                Err(InstallError::Transient) => {
                    sleep_ms(RETRY_SLEEP_MS);
                    attempt += 1;
                }
            }
        }
    }
    let mut detail = reasons.join(";");
    let mut end = detail.len().min(MAX_DETAIL);
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail.truncate(end);
    Err(InstallError::Unsupported(detail))
}

/// Waits until no detour is running and then a grace period more (a thread may just have
/// left it). `Err` if one is still running after [`DRAIN_MAX_MS`].
pub fn drain(in_detour: &dyn Fn() -> usize, clock: &dyn Clock) -> Result<(), String> {
    let start = clock.now_ms();
    loop {
        if in_detour() == 0 {
            clock.sleep_ms(DRAIN_GRACE_MS);
            if in_detour() == 0 {
                return Ok(());
            }
        }
        if clock.now_ms().saturating_sub(start) >= DRAIN_MAX_MS {
            return Err(format!("{} detour(s) still running", in_detour()));
        }
        clock.sleep_ms(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slot::fake_clock::FakeClock;
    use std::cell::{Cell, RefCell};

    const WINDOW: std::ops::Range<usize> = 0x1000..0x100E;

    #[test]
    fn patch_blocked_by_an_ip_in_the_window() {
        assert!(!patch_blocked(&[], WINDOW));
        assert!(!patch_blocked(&[0x0FFF, 0x100E], WINDOW));
        assert!(patch_blocked(&[0x1000], WINDOW));
        assert!(patch_blocked(&[0x100D], WINDOW));
        assert!(patch_blocked(&[0x5, 0x1004, 0x9], WINDOW));
    }

    #[test]
    fn unhook_blocked_also_by_trampoline_dll_and_detour() {
        let dll = 0x9000..0xA000;
        let blocked =
            |ips: &[usize], in_detour| unhook_blocked(ips, WINDOW, 0x5000, dll.clone(), in_detour);
        assert!(!blocked(&[], 0));
        assert!(!blocked(&[0x0FFF, 0x5400, 0xA000], 0));
        assert!(blocked(&[0x5000], 0));
        assert!(blocked(&[0x53FF], 0));
        assert!(blocked(&[0x9000], 0));
        assert!(blocked(&[0x9FFF], 0));
        assert!(blocked(&[0x1000], 0));
        assert!(blocked(&[], 1));
    }

    #[test]
    fn decode_jmp_target_reads_both_jump_forms() {
        let mut near = vec![0xE9];
        near.extend(0x100i32.to_le_bytes());
        assert_eq!(decode_jmp_target(&near, 0x1000), Some(0x1105));
        let mut back = vec![0xE9];
        back.extend((-0x10i32).to_le_bytes());
        assert_eq!(decode_jmp_target(&back, 0x1000), Some(0x1000 + 5 - 0x10));
        let mut far = vec![0xFF, 0x25, 0, 0, 0, 0];
        far.extend(0x7FF6_1234_5678u64.to_le_bytes());
        assert_eq!(decode_jmp_target(&far, 0x1000), Some(0x7FF6_1234_5678));
        assert_eq!(decode_jmp_target(&[0x90; 14], 0x1000), None);
        assert_eq!(decode_jmp_target(&[0xE9, 1, 2], 0x1000), None);
        assert_eq!(decode_jmp_target(&[], 0x1000), None);
        assert_eq!(decode_jmp_target(&far[..10], 0x1000), None);
    }

    #[test]
    fn threads_of_filters_and_caps() {
        let entries = [(7, 1), (8, 2), (7, 3), (7, 4), (9, 5)];
        assert_eq!(threads_of(&entries, 7, 3, 10), Ok(vec![1, 4]));
        assert_eq!(threads_of(&entries, 7, 3, 2), Ok(vec![1, 4]));
        assert_eq!(threads_of(&entries, 7, 3, 1), Err(TooManyThreads));
        assert_eq!(threads_of(&[], 7, 3, 1), Ok(vec![]));
    }

    #[test]
    fn the_main_thread_is_the_earliest_created() {
        assert_eq!(pick_main_thread(&[(5, 300), (6, 100), (7, 200)]), Some(6));
        assert_eq!(pick_main_thread(&[]), None);
    }

    fn run(
        candidates: &[&str],
        script: &mut Vec<(&str, Result<u8, InstallError>)>,
    ) -> (Result<(String, u8), InstallError>, Vec<String>, Vec<u64>) {
        let attempts = RefCell::new(Vec::new());
        let sleeps = RefCell::new(Vec::new());
        let result = install_with_retry(
            candidates,
            |name| {
                attempts.borrow_mut().push(name.to_string());
                let at = script.iter().position(|(n, _)| *n == name).unwrap();
                script[at].1.clone()
            },
            |ms| sleeps.borrow_mut().push(ms),
        );
        (result, attempts.into_inner(), sleeps.into_inner())
    }

    #[test]
    fn transient_twice_then_ok() {
        let results = RefCell::new(vec![
            Err(InstallError::Transient),
            Err(InstallError::Transient),
            Ok(7u8),
        ]);
        let sleeps = RefCell::new(Vec::new());
        let r = install_with_retry(
            &["a"],
            |_| results.borrow_mut().remove(0),
            |ms| sleeps.borrow_mut().push(ms),
        );
        assert_eq!(r, Ok(("a".to_string(), 7)));
        assert_eq!(sleeps.into_inner(), vec![2, 2]);
        assert!(results.borrow().is_empty());
    }

    #[test]
    fn transient_always_stops_after_five_attempts() {
        let (r, attempts, sleeps) = run(
            &["a", "b"],
            &mut vec![("a", Err(InstallError::Transient)), ("b", Ok(1))],
        );
        assert_eq!(r, Err(InstallError::Transient));
        assert_eq!(attempts.len(), 5);
        assert_eq!(sleeps, vec![2; 4]);
    }

    #[test]
    fn unsupported_moves_to_the_next_candidate() {
        let (r, attempts, sleeps) = run(
            &["a", "b"],
            &mut vec![
                (
                    "a",
                    Err(InstallError::Unsupported("MovingCodeNotSupported".into())),
                ),
                ("b", Ok(2)),
            ],
        );
        assert_eq!(r, Ok(("b".to_string(), 2)));
        assert_eq!(attempts, vec!["a", "b"]);
        assert!(sleeps.is_empty());
    }

    #[test]
    fn unsupported_everywhere_names_every_candidate() {
        let (r, attempts, _) = run(
            &["ZRpcCtrl.Update", "Game.Update"],
            &mut vec![
                (
                    "ZRpcCtrl.Update",
                    Err(InstallError::Unsupported("Disassemble".into())),
                ),
                (
                    "Game.Update",
                    Err(InstallError::Unsupported("method_missing:pump".into())),
                ),
            ],
        );
        assert_eq!(attempts.len(), 2);
        match r {
            Err(InstallError::Unsupported(d)) => {
                assert!(
                    d.contains("ZRpcCtrl.Update") && d.contains("Game.Update"),
                    "{d}"
                );
                assert!(
                    d.contains("Disassemble") && d.contains("method_missing:pump"),
                    "{d}"
                );
                assert!(d.len() <= 256);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_long_unsupported_detail_is_cut() {
        let long = "x".repeat(400);
        let (r, _, _) = run(
            &["a"],
            &mut vec![("a", Err(InstallError::Unsupported(long)))],
        );
        match r {
            Err(InstallError::Unsupported(d)) => assert!(d.len() <= 256),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn drain_waits_the_grace_when_nothing_is_in_flight() {
        let counter = Cell::new(0usize);
        let clock = FakeClock::new();
        assert_eq!(drain(&|| counter.get(), &clock), Ok(()));
        assert_eq!(clock.now(), 50);
    }

    #[test]
    fn drain_waits_for_a_detour_then_the_grace() {
        let counter = Cell::new(1usize);
        let clock = FakeClock::new().at(300, || counter.set(0));
        assert_eq!(drain(&|| counter.get(), &clock), Ok(()));
        assert_eq!(clock.now(), 350);
    }

    #[test]
    fn drain_gives_up_when_stuck() {
        let counter = Cell::new(1usize);
        let clock = FakeClock::new();
        assert!(drain(&|| counter.get(), &clock).is_err());
        assert_eq!(clock.now(), 2000);
    }
}
