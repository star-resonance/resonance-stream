//! The pure half of in-game chat sending: which process is the game, whether the
//! DLL file is the one the app was built with, the Korean sentence of every
//! failure, and (further down) the pipe exchange, the load sequence and the send
//! flow, all behind small traits so they run on any OS. The Win32 calls that
//! implement the traits are in `src-tauri/src/ingame/win/`.
//!
//! Chat text is never logged here: only the channel and the length.

use crate::send_text::SendSpacing;
use resonance_types::ingame_pipe::{
    decode_reply, encode_request, ErrorCode, LineBuffer, Reply, Request, PIPE_PROTOCOL_VERSION,
    WRITE_TIMEOUT_MS,
};
use resonance_types::{prepare_text, SendChatChannel, SendTextError};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Read};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Executable base names that are the game: the stem starts with one of these.
pub const GAME_EXE_PREFIXES: [&str; 2] = ["StarSEA", "StarASIA"];
/// The game's chat server port; the game process owns a connection to it.
pub const CHAT_PORT: u16 = 5003;
/// The DLL's file name, in the game and next to the app.
pub const DLL_NAME: &str = "resonance_ingame.dll";
/// How long the app waits for the DLL to connect to the pipe after the load.
pub const CONNECT_WAIT: Duration = Duration::from_secs(5);
/// How long the app waits for the remote `LoadLibraryW` thread.
pub const LOAD_WAIT: Duration = Duration::from_secs(5);
/// How long the app waits for the module to leave the game after `bye`.
pub const UNLOAD_WAIT: Duration = Duration::from_secs(2);

/// One running process, as the process snapshot lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    /// The executable's file name (`StarSEA.exe`).
    pub exe: String,
}

/// Why no single 64-bit game process could be chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameError {
    NotRunning,
    Several,
    Is32Bit,
}

/// True for `<prefix>*.exe` (case-insensitive), a prefix of [`GAME_EXE_PREFIXES`].
pub fn is_game_exe(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".exe") else {
        return false;
    };
    GAME_EXE_PREFIXES
        .iter()
        .any(|p| stem.starts_with(&p.to_ascii_lowercase()))
}

/// Lists the pids of the running 64-bit game clients, sorted. `port_owners` (the
/// pids holding a connection to [`CHAT_PORT`]) is asked for only when more than one
/// exe matches; then only the owners are clients. A 32-bit game is dropped, and a
/// list that was not empty and is now is `Is32Bit`.
pub fn list_games(
    processes: &[ProcessEntry],
    port_owners: impl FnOnce() -> Vec<u32>,
    is_32bit: impl Fn(u32) -> bool,
) -> Result<Vec<u32>, GameError> {
    let mut pids: Vec<u32> = processes
        .iter()
        .filter(|p| is_game_exe(&p.exe))
        .map(|p| p.pid)
        .collect();
    pids.sort_unstable();
    pids.dedup();
    if pids.len() > 1 {
        let owners = port_owners();
        pids.retain(|p| owners.contains(p));
        if pids.is_empty() {
            return Err(GameError::Several);
        }
    }
    if pids.is_empty() {
        return Err(GameError::NotRunning);
    }
    pids.retain(|p| !is_32bit(*p));
    if pids.is_empty() {
        return Err(GameError::Is32Bit);
    }
    Ok(pids)
}

/// The client a send goes to: the only one, or the one `target` names. Several
/// clients and no target is `ChooseClient`; a target that is not listed is `ClientGone`.
pub fn pick_target(clients: &[u32], target: Option<u32>) -> Result<u32, SendError> {
    match (clients, target) {
        ([one], None) => Ok(*one),
        (_, Some(pid)) if clients.contains(&pid) => Ok(pid),
        (_, Some(_)) => Err(SendError::ClientGone),
        _ => Err(SendError::ChooseClient),
    }
}

/// The port of a TCP table row's `dwRemotePort`: the low 16 bits, in network order.
pub fn remote_port(raw: u32) -> u16 {
    (raw as u16).swap_bytes()
}

/// SHA-256 of everything `reader` yields, as lower-case hex.
pub fn sha256_hex(mut reader: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Why the DLL file is not loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinError {
    /// The app was built without a DLL (or with an unusable pin).
    NoPin,
    /// The file's hash is not the pin.
    Mismatch,
}

/// Compares the build-time pin with the file's hash. A pin that is not 64 hex
/// digits counts as no pin.
pub fn check_pin(pin: Option<&str>, actual: &str) -> Result<(), PinError> {
    let pin = match pin {
        Some(p) if p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit()) => p,
        _ => return Err(PinError::NoPin),
    };
    if pin.eq_ignore_ascii_case(actual) {
        Ok(())
    } else {
        Err(PinError::Mismatch)
    }
}

/// The pipe's security descriptor: SYSTEM and Administrators full, the current
/// user read/write, a Medium mandatory label so a Medium-integrity game can connect.
pub fn pipe_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{user_sid})S:(ML;;NW;;;ME)")
}

/// The sentence for a DLL error code (selected by code, never by its `detail`).
pub fn code_text(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::BadRequest => {
            "게임 내 전송 모듈과 통신하지 못했습니다. 앱을 다시 시작해 주세요."
        }
        ErrorCode::BadChannel => "이 채널로는 보낼 수 없습니다.",
        ErrorCode::Empty => "보낼 내용이 없습니다.",
        ErrorCode::TooLong => "메시지가 너무 깁니다. 140자 이하로 줄여 주세요.",
        ErrorCode::NoClient => {
            "게임 채팅 서버에 연결되어 있지 않습니다. 로그인한 뒤 다시 시도해 주세요."
        }
        ErrorCode::NotSent => "게임이 메시지를 보내지 못했습니다.",
        ErrorCode::Busy => "이전 메시지를 보내는 중입니다. 잠시 후 다시 시도해 주세요.",
        ErrorCode::Expired => {
            "게임이 응답하지 않아 보내지 않았습니다. 로딩이 끝난 뒤 다시 시도해 주세요."
        }
        ErrorCode::Unsupported => {
            "이 게임 버전에서는 채팅 보내기를 지원하지 않습니다. 앱 업데이트를 기다려 주세요."
        }
        ErrorCode::Version => "게임 내 전송 모듈의 버전이 맞지 않습니다. 앱을 다시 시작해 주세요.",
    }
}

/// Everything that can stop a send; `Display` is the one Korean sentence the ui shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendError {
    SettingOff,
    Game(GameError),
    /// Several clients are connected and the call named none.
    ChooseClient,
    /// The named client is no longer running.
    ClientGone,
    NoPin,
    /// The module of this version is being downloaded; it can be used when that ends.
    DllDownloading,
    DllMissing,
    PinMismatch,
    OpenFailed,
    LoadFailed,
    PipeCreate,
    NoConnect,
    Broken,
    /// The module stayed in the game after the unload wait (`KeepResident`).
    ResidentModule,
    /// The DLL answered with an error.
    Dll(ErrorCode),
    /// The text rules or the spacing rule.
    Text(SendTextError),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            SendError::SettingOff => "게임 내 채팅 보내기가 꺼져 있습니다. 설정에서 켜 주세요.",
            SendError::Game(GameError::NotRunning) => "게임이 실행 중이 아닙니다.",
            SendError::Game(GameError::Several) => "게임이 둘 이상 실행 중입니다.",
            SendError::Game(GameError::Is32Bit) => "32비트 게임은 지원하지 않습니다.",
            SendError::ChooseClient => "게임이 둘 이상 연결되어 있습니다. 보낼 게임을 골라 주세요.",
            SendError::ClientGone => "선택한 게임을 찾을 수 없습니다. 다시 골라 주세요.",
            SendError::NoPin => "이 앱에는 게임 내 전송 모듈이 들어 있지 않습니다.",
            SendError::DllDownloading => {
                "게임 내 전송 모듈을 내려받는 중입니다. 끝나면 보낼 수 있습니다."
            }
            SendError::DllMissing => {
                "게임 내 전송 모듈 파일이 없습니다. 설정에서 껐다 켜면 다시 내려받습니다."
            }
            SendError::PinMismatch => {
                "게임 내 전송 모듈 파일이 변경되어 불러오지 않았습니다. 설정에서 껐다 켜면 다시 내려받습니다."
            }
            SendError::OpenFailed => "게임 프로세스에 접근하지 못했습니다.",
            SendError::LoadFailed => "게임에 전송 모듈을 불러오지 못했습니다.",
            SendError::PipeCreate => {
                "게임 내 전송 모듈과 통신할 수 없습니다. 앱을 다시 시작해 주세요."
            }
            SendError::NoConnect => "게임 내 전송 모듈이 연결되지 않았습니다.",
            SendError::Broken => "게임 내 전송 모듈이 응답하지 않습니다. 설정에서 껐다 켜 주세요.",
            SendError::ResidentModule => {
                "게임에 전송 모듈이 남아 있어 다시 불러올 수 없습니다. 게임을 다시 시작해 주세요."
            }
            SendError::Dll(code) => code_text(*code),
            SendError::Text(e) => return write!(f, "{e}"),
        };
        f.write_str(text)
    }
}

impl std::error::Error for SendError {}

impl From<GameError> for SendError {
    fn from(e: GameError) -> Self {
        SendError::Game(e)
    }
}

impl From<SendTextError> for SendError {
    fn from(e: SendTextError) -> Self {
        SendError::Text(e)
    }
}

impl From<PinError> for SendError {
    fn from(e: PinError) -> Self {
        match e {
            PinError::NoPin => SendError::NoPin,
            PinError::Mismatch => SendError::PinMismatch,
        }
    }
}

// ---- the pipe exchange ------------------------------------------------------

/// Why a pipe read or write did not finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Timeout,
    Broken,
}

/// The app's end of the pipe. The Windows implementation does overlapped I/O and
/// cancels it on a timeout.
pub trait Wire {
    /// Writes the whole `line`, or fails after `timeout`.
    fn write_line(&mut self, line: &[u8], timeout: Duration) -> Result<(), WireError>;
    /// Reads at least one byte (0 means the pipe closed), or fails after `wait`.
    fn read(&mut self, buf: &mut [u8], wait: Duration) -> Result<usize, WireError>;
}

fn reply_id(reply: &Reply) -> u32 {
    match reply {
        Reply::Hello { id, .. }
        | Reply::Send { id, .. }
        | Reply::Ack { id }
        | Reply::Error { id, .. } => *id,
    }
}

/// Sends one request and reads its reply, all within `wait`. A write or read that
/// fails, a reply that does not decode, is too long or answers another request is
/// `Broken` (the session is dropped). A DLL `Reply::Error` is returned as `Ok`: the
/// caller maps it.
pub fn exchange(
    wire: &mut impl Wire,
    request: &Request,
    wait: Duration,
) -> Result<Reply, SendError> {
    let line = encode_request(request);
    wire.write_line(
        line.as_bytes(),
        Duration::from_millis(u64::from(WRITE_TIMEOUT_MS)),
    )
    .map_err(|_| SendError::Broken)?;
    let started = Instant::now();
    let mut lines = LineBuffer::new();
    let mut buf = [0u8; 4096];
    loop {
        let left = wait
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or(SendError::Broken)?;
        let count = wire.read(&mut buf, left).map_err(|_| SendError::Broken)?;
        if count == 0 {
            return Err(SendError::Broken);
        }
        let got = lines.push(&buf[..count]).map_err(|_| SendError::Broken)?;
        if let Some(first) = got.first() {
            let reply = decode_reply(first).map_err(|_| SendError::Broken)?;
            if reply_id(&reply) != request.id() {
                return Err(SendError::Broken);
            }
            return Ok(reply);
        }
    }
}

/// Waits for clients of the pipe.
pub trait Listener {
    /// Waits up to `wait` for a client; its process id, or `None` on timeout.
    fn wait_client(&mut self, wait: Duration) -> Result<Option<u32>, SendError>;
    /// Drops the current client so the wait can go on.
    fn disconnect(&mut self);
}

/// Waits up to `total` for the game process itself to connect: a client of any
/// other process is disconnected and the wait goes on.
pub fn accept_game(
    listener: &mut impl Listener,
    game_pid: u32,
    total: Duration,
) -> Result<(), SendError> {
    let started = Instant::now();
    loop {
        let left = total
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or(SendError::NoConnect)?;
        match listener.wait_client(left)? {
            None => return Err(SendError::NoConnect),
            Some(pid) if pid == game_pid => return Ok(()),
            Some(_) => listener.disconnect(),
        }
    }
}

// ---- the load sequence --------------------------------------------------------

/// The steps of connecting to the game, so the order can be tested; the Windows
/// implementation is in `src-tauri/src/ingame/win/`.
pub trait Host {
    /// Creates the pipe (first instance) for `pid`; before any load.
    fn create_pipe(&mut self, pid: u32) -> Result<(), SendError>;
    /// Whether the DLL module is in the game's module list.
    fn module_loaded(&mut self, pid: u32) -> bool;
    /// Waits up to `wait` for the module to leave; true once it has.
    fn wait_module_gone(&mut self, pid: u32, wait: Duration) -> bool;
    /// Opens and holds the DLL file and checks its hash against the pin.
    fn verify_dll(&mut self) -> Result<(), SendError>;
    /// Loads the verified file into `pid` ([`LOAD_WAIT`]).
    fn load_dll(&mut self, pid: u32) -> Result<(), SendError>;
    /// Waits for the game to connect to the pipe ([`accept_game`]).
    fn accept(&mut self, pid: u32) -> Result<(), SendError>;
    /// Sends `hello` with the protocol version and returns the reply.
    fn hello(&mut self) -> Result<Reply, SendError>;
    /// Sends `bye` and waits up to [`UNLOAD_WAIT`] for the module to leave; false
    /// when it is still loaded (the DLL kept itself resident).
    fn bye_and_wait(&mut self, pid: u32) -> bool;
}

/// The game processes whose module stayed loaded after the unload wait. A resident
/// module is not reopened by a second load, so while it is there the app never
/// loads it again; a game with another pid (a restart) loads it fresh.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Resident {
    pids: BTreeSet<u32>,
}

impl Resident {
    /// Empty, usable in a `static`.
    pub const fn new() -> Self {
        Resident {
            pids: BTreeSet::new(),
        }
    }
    pub fn note(&mut self, pid: u32) {
        self.pids.insert(pid);
    }
    pub fn holds(&self, pid: u32) -> bool {
        self.pids.contains(&pid)
    }
    /// Forgets `pid` only.
    pub fn clear(&mut self, pid: u32) {
        self.pids.remove(&pid);
    }
}

/// Creates the pipe for `pid`, loads the DLL and says `hello`; returns `pid`. A DLL of another protocol version (a stale module: the file name is
/// fixed, so the old one must leave first) gets one `bye`, a wait and a second try.
///
/// A module that is still in the game after [`UNLOAD_WAIT`] is resident: it is
/// noted in `resident`, and `connect` returns `ResidentModule` without loading again.
pub fn connect(host: &mut impl Host, resident: &mut Resident, pid: u32) -> Result<u32, SendError> {
    if resident.holds(pid) {
        if host.module_loaded(pid) {
            return Err(SendError::ResidentModule);
        }
        resident.clear(pid);
    }
    host.create_pipe(pid).map_err(|_| SendError::PipeCreate)?;
    if host.module_loaded(pid) && !host.wait_module_gone(pid, UNLOAD_WAIT) {
        resident.note(pid);
        return Err(SendError::ResidentModule);
    }
    let mut replaced = false;
    loop {
        host.verify_dll()?;
        host.load_dll(pid)?;
        host.accept(pid)?;
        match host.hello()? {
            Reply::Hello { version, .. } if version == PIPE_PROTOCOL_VERSION => return Ok(pid),
            Reply::Hello { .. }
            | Reply::Error {
                code: ErrorCode::Version,
                ..
            } => {
                if replaced {
                    return Err(SendError::Dll(ErrorCode::Version));
                }
                replaced = true;
                if !host.bye_and_wait(pid) {
                    resident.note(pid);
                    return Err(SendError::ResidentModule);
                }
            }
            Reply::Error { code, detail, .. } => {
                log::warn!("hello refused: {} ({detail})", code.as_str());
                return Err(SendError::Dll(code));
            }
            _ => return Err(SendError::Broken),
        }
    }
}

// ---- the send flow ------------------------------------------------------------

/// Maps the DLL's reply to a `send` request to the result and whether the session
/// must be dropped (`bad_request` and `version` mean the two ends no longer
/// understand each other; so does any reply that is no answer to a send). The
/// `detail` goes to the log with the code, never the chat text.
pub fn map_send_reply(reply: Reply) -> (Result<(), SendError>, bool) {
    match reply {
        Reply::Send { .. } | Reply::Ack { .. } => (Ok(()), false),
        Reply::Error { code, detail, .. } => {
            log::warn!("in-game send refused: {} ({detail})", code.as_str());
            let drop = matches!(code, ErrorCode::BadRequest | ErrorCode::Version);
            (Err(SendError::Dll(code)), drop)
        }
        Reply::Hello { .. } => (Err(SendError::Broken), true),
    }
}

/// What `send_chat` needs from the session: connect if needed, send, map the reply.
pub trait SendBackend {
    /// `target` is the game's pid (never shown to the user); `None` is allowed only
    /// with one client.
    fn send(
        &mut self,
        target: Option<u32>,
        channel: SendChatChannel,
        text: &str,
    ) -> Result<(), SendError>;
}

/// The `send_chat` command's logic: setting, text rules, spacing, send. The spacing
/// guard is held across the send, so two calls at once cannot both pass; only a
/// send that succeeded starts the window. The error is the one sentence the ui shows.
/// Chat text is not logged, only the channel and the length.
pub fn send_chat_flow(
    setting_on: bool,
    spacing: &Mutex<SendSpacing>,
    backend: &mut impl SendBackend,
    target: Option<u32>,
    channel: SendChatChannel,
    text: &str,
) -> Result<(), String> {
    let mut run = || -> Result<(), SendError> {
        if !setting_on {
            return Err(SendError::SettingOff);
        }
        let prepared = prepare_text(text)?;
        let mut guard = spacing.lock().unwrap_or_else(PoisonError::into_inner);
        guard.allow(Instant::now())?;
        backend.send(target, channel, &prepared)?;
        guard.record_success(Instant::now());
        log::debug!(
            "chat sent: channel {}, {} units",
            channel.as_str(),
            prepared.encode_utf16().count()
        );
        Ok(())
    };
    run().map_err(|e| {
        log::debug!("chat not sent: channel {}: {e}", channel.as_str());
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_types::{ingame_pipe::ErrorCode, SendTextError};

    fn proc(pid: u32, exe: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            exe: exe.to_string(),
        }
    }

    #[test]
    fn game_exe_names_match_by_prefix_and_extension() {
        for name in [
            "StarSEA.exe",
            "StarSEA_STEAM.exe",
            "StarASIA.exe",
            "starasia.EXE",
        ] {
            assert!(is_game_exe(name), "{name}");
        }
        for name in ["StarSEA", "notepad.exe", "MyStarSEA.exe", ""] {
            assert!(!is_game_exe(name), "{name}");
        }
    }

    fn run(
        list: &[ProcessEntry],
        owners: Vec<u32>,
        is_32bit: impl Fn(u32) -> bool,
    ) -> Result<Vec<u32>, GameError> {
        list_games(list, || owners, is_32bit)
    }

    #[test]
    fn list_games_with_no_match_does_not_ask_for_port_owners() {
        let list = [proc(1, "notepad.exe")];
        let got = list_games(&list, || panic!("asked"), |_| false);
        assert_eq!(got, Err(GameError::NotRunning));
        assert_eq!(
            list_games(&[], || panic!("asked"), |_| false),
            Err(GameError::NotRunning)
        );
    }

    #[test]
    fn list_games_with_one_match_returns_it_without_asking() {
        let list = [proc(5, "notepad.exe"), proc(10, "StarSEA.exe")];
        assert_eq!(
            list_games(&list, || panic!("asked"), |_| false),
            Ok(vec![10])
        );
    }

    #[test]
    fn list_games_with_several_matches_lists_the_port_owners_sorted() {
        let list = [proc(42, "StarSEA.exe"), proc(7, "StarASIA.exe")];
        assert_eq!(run(&list, vec![42, 7], |_| false), Ok(vec![7, 42]));
        assert_eq!(run(&list, vec![42], |_| false), Ok(vec![42]));
        assert_eq!(run(&list, Vec::new(), |_| false), Err(GameError::Several));
        // an owner that is no game process does not count
        assert_eq!(run(&list, vec![99, 42], |_| false), Ok(vec![42]));
    }

    #[test]
    fn list_games_drops_32_bit_games() {
        let one = [proc(10, "StarSEA.exe")];
        assert_eq!(
            list_games(&one, || panic!("asked"), |pid| pid == 10),
            Err(GameError::Is32Bit)
        );
        let two = [proc(42, "StarSEA.exe"), proc(7, "StarSEA.exe")];
        assert_eq!(run(&two, vec![42, 7], |pid| pid == 7), Ok(vec![42]));
        assert_eq!(run(&two, vec![42, 7], |_| true), Err(GameError::Is32Bit));
    }

    #[test]
    fn list_games_ignores_non_game_processes_with_equal_pids() {
        let list = [proc(10, "notepad.exe"), proc(20, "StarSEA.exe")];
        assert_eq!(run(&list, vec![10], |_| false), Ok(vec![20]));
    }

    #[test]
    fn pick_target_takes_the_only_client_or_the_named_one() {
        assert_eq!(pick_target(&[42], None), Ok(42));
        assert_eq!(pick_target(&[42], Some(42)), Ok(42));
        assert_eq!(pick_target(&[42], Some(7)), Err(SendError::ClientGone));
        assert_eq!(pick_target(&[7, 42], Some(7)), Ok(7));
        assert_eq!(pick_target(&[7, 42], None), Err(SendError::ChooseClient));
        assert_eq!(pick_target(&[7, 42], Some(9)), Err(SendError::ClientGone));
    }

    #[test]
    fn the_client_sentences_are_one_line_without_digits_and_their_own() {
        let choose = SendError::ChooseClient.to_string();
        let gone = SendError::ClientGone.to_string();
        assert_eq!(
            choose,
            "게임이 둘 이상 연결되어 있습니다. 보낼 게임을 골라 주세요."
        );
        assert_eq!(gone, "선택한 게임을 찾을 수 없습니다. 다시 골라 주세요.");
        for text in [&choose, &gone] {
            assert!(!text.contains('\n') && !text.chars().any(|c| c.is_ascii_digit()));
        }
        assert_ne!(choose, gone);
        for other in [
            SendError::SettingOff,
            SendError::Game(GameError::Several),
            SendError::NoConnect,
            SendError::Broken,
            SendError::ResidentModule,
        ] {
            assert_ne!(other.to_string(), choose);
            assert_ne!(other.to_string(), gone);
        }
    }

    #[test]
    fn the_resident_memory_holds_a_set_of_pids() {
        let mut resident = Resident::new();
        assert!(!resident.holds(42) && !resident.holds(7));
        resident.note(42);
        resident.note(7);
        assert!(resident.holds(42) && resident.holds(7));
        resident.clear(42);
        assert!(!resident.holds(42) && resident.holds(7));
    }

    #[test]
    fn remote_port_reads_the_low_16_bits_in_network_order() {
        assert_eq!(remote_port(0x8B13), 5003);
        assert_eq!(remote_port(0xFFFF_8B13), 5003);
        assert_eq!(remote_port(0x5000), 80);
    }

    #[test]
    fn sha256_hex_is_lower_case_hex() {
        assert_eq!(
            sha256_hex(&b"abc"[..]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(&b""[..]).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    const PIN: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn check_pin_compares_hex_case_insensitively() {
        assert_eq!(check_pin(Some(PIN), PIN), Ok(()));
        assert_eq!(check_pin(Some(&PIN.to_uppercase()), PIN), Ok(()));
        let other = PIN.replacen('b', "c", 1);
        assert_eq!(check_pin(Some(&other), PIN), Err(PinError::Mismatch));
    }

    #[test]
    fn check_pin_without_a_usable_pin_is_no_pin() {
        assert_eq!(check_pin(None, PIN), Err(PinError::NoPin));
        assert_eq!(check_pin(Some(""), PIN), Err(PinError::NoPin));
        assert_eq!(check_pin(Some(&PIN[..63]), PIN), Err(PinError::NoPin));
        assert_eq!(
            check_pin(Some(&PIN.replace('a', "z")), PIN),
            Err(PinError::NoPin)
        );
    }

    #[test]
    fn the_pipe_sddl_is_exact() {
        assert_eq!(
            pipe_sddl("S-1-5-21-1-2-3-1001"),
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;S-1-5-21-1-2-3-1001)S:(ML;;NW;;;ME)"
        );
    }

    #[test]
    fn code_text_is_the_ten_sentences_in_order() {
        let want = [
            "게임 내 전송 모듈과 통신하지 못했습니다. 앱을 다시 시작해 주세요.",
            "이 채널로는 보낼 수 없습니다.",
            "보낼 내용이 없습니다.",
            "메시지가 너무 깁니다. 140자 이하로 줄여 주세요.",
            "게임 채팅 서버에 연결되어 있지 않습니다. 로그인한 뒤 다시 시도해 주세요.",
            "게임이 메시지를 보내지 못했습니다.",
            "이전 메시지를 보내는 중입니다. 잠시 후 다시 시도해 주세요.",
            "게임이 응답하지 않아 보내지 않았습니다. 로딩이 끝난 뒤 다시 시도해 주세요.",
            "이 게임 버전에서는 채팅 보내기를 지원하지 않습니다. 앱 업데이트를 기다려 주세요.",
            "게임 내 전송 모듈의 버전이 맞지 않습니다. 앱을 다시 시작해 주세요.",
        ];
        let got: Vec<&str> = ErrorCode::ALL.into_iter().map(code_text).collect();
        assert_eq!(got, want);
        for (i, a) in got.iter().enumerate() {
            assert!(!a.is_empty() && !a.contains('\n'));
            assert!(got[i + 1..].iter().all(|b| a != b));
        }
        for code in ErrorCode::ALL {
            assert_eq!(SendError::Dll(code).to_string(), code_text(code));
        }
    }

    #[test]
    fn every_send_error_displays_its_sentence() {
        let table = [
            (
                SendError::SettingOff,
                "게임 내 채팅 보내기가 꺼져 있습니다. 설정에서 켜 주세요.",
            ),
            (
                SendError::Game(GameError::NotRunning),
                "게임이 실행 중이 아닙니다.",
            ),
            (
                SendError::Game(GameError::Several),
                "게임이 둘 이상 실행 중입니다.",
            ),
            (
                SendError::Game(GameError::Is32Bit),
                "32비트 게임은 지원하지 않습니다.",
            ),
            (
                SendError::NoPin,
                "이 앱에는 게임 내 전송 모듈이 들어 있지 않습니다.",
            ),
            (
                SendError::DllDownloading,
                "게임 내 전송 모듈을 내려받는 중입니다. 끝나면 보낼 수 있습니다.",
            ),
            (
                SendError::DllMissing,
                "게임 내 전송 모듈 파일이 없습니다. 설정에서 껐다 켜면 다시 내려받습니다.",
            ),
            (
                SendError::PinMismatch,
                "게임 내 전송 모듈 파일이 변경되어 불러오지 않았습니다. 설정에서 껐다 켜면 다시 내려받습니다.",
            ),
            (
                SendError::OpenFailed,
                "게임 프로세스에 접근하지 못했습니다.",
            ),
            (
                SendError::LoadFailed,
                "게임에 전송 모듈을 불러오지 못했습니다.",
            ),
            (
                SendError::PipeCreate,
                "게임 내 전송 모듈과 통신할 수 없습니다. 앱을 다시 시작해 주세요.",
            ),
            (
                SendError::NoConnect,
                "게임 내 전송 모듈이 연결되지 않았습니다.",
            ),
            (
                SendError::Broken,
                "게임 내 전송 모듈이 응답하지 않습니다. 설정에서 껐다 켜 주세요.",
            ),
            (SendError::ResidentModule, RESIDENT_SENTENCE),
        ];
        for (err, sentence) in table {
            assert_eq!(err.to_string(), sentence);
            assert!(!sentence.contains('\n'));
            assert!(!sentence.contains("다시 설치"), "{sentence}");
        }
        assert_eq!(
            SendError::Text(SendTextError::Empty).to_string(),
            SendTextError::Empty.to_string()
        );
    }

    // ---- flows -------------------------------------------------------------

    use crate::send_text::ClientSpacing;
    use resonance_types::ingame_pipe::decode_request;
    use std::collections::VecDeque;
    use std::sync::Arc;

    struct FakeWire {
        writes: Vec<(Vec<u8>, Duration)>,
        write_result: Result<(), WireError>,
        chunks: VecDeque<Result<Vec<u8>, WireError>>,
    }

    impl FakeWire {
        fn replying(chunks: Vec<Result<Vec<u8>, WireError>>) -> Self {
            FakeWire {
                writes: Vec::new(),
                write_result: Ok(()),
                chunks: chunks.into(),
            }
        }
        fn line(s: &str) -> Self {
            Self::replying(vec![Ok(s.as_bytes().to_vec())])
        }
    }

    impl Wire for FakeWire {
        fn write_line(&mut self, line: &[u8], timeout: Duration) -> Result<(), WireError> {
            self.writes.push((line.to_vec(), timeout));
            self.write_result
        }
        fn read(&mut self, buf: &mut [u8], _wait: Duration) -> Result<usize, WireError> {
            match self.chunks.pop_front() {
                None => Err(WireError::Timeout),
                Some(Err(e)) => Err(e),
                Some(Ok(mut bytes)) => {
                    let n = bytes.len().min(buf.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                    if n < bytes.len() {
                        self.chunks.push_front(Ok(bytes.split_off(n)));
                    }
                    Ok(n)
                }
            }
        }
    }

    fn ping(id: u32) -> Request {
        Request::Ping { id }
    }

    const WAIT: Duration = Duration::from_millis(1500);

    #[test]
    fn exchange_writes_the_encoded_request_with_the_write_deadline() {
        let mut wire = FakeWire::line("{\"id\":7,\"ok\":true}\n");
        let reply = exchange(&mut wire, &ping(7), WAIT).unwrap();
        assert_eq!(reply, Reply::Ack { id: 7 });
        assert_eq!(wire.writes.len(), 1);
        assert_eq!(wire.writes[0].0, encode_request(&ping(7)).into_bytes());
        assert_eq!(
            wire.writes[0].1,
            Duration::from_millis(WRITE_TIMEOUT_MS as u64)
        );
        assert_eq!(
            decode_request(&wire.writes[0].0[..wire.writes[0].0.len() - 1]),
            Ok(ping(7))
        );
    }

    #[test]
    fn exchange_joins_a_reply_split_across_reads() {
        let mut wire = FakeWire::replying(vec![
            Ok(b"{\"id\":7,".to_vec()),
            Ok(b"\"ok\":true}\n".to_vec()),
        ]);
        assert_eq!(
            exchange(&mut wire, &ping(7), WAIT),
            Ok(Reply::Ack { id: 7 })
        );
    }

    #[test]
    fn exchange_breaks_on_another_id_a_timeout_or_a_failed_write() {
        let mut wire = FakeWire::line("{\"id\":8,\"ok\":true}\n");
        assert_eq!(exchange(&mut wire, &ping(7), WAIT), Err(SendError::Broken));
        let mut wire = FakeWire::replying(vec![Err(WireError::Timeout)]);
        assert_eq!(exchange(&mut wire, &ping(7), WAIT), Err(SendError::Broken));
        let mut wire = FakeWire::line("{\"id\":7,\"ok\":true}\n");
        wire.write_result = Err(WireError::Broken);
        assert_eq!(exchange(&mut wire, &ping(7), WAIT), Err(SendError::Broken));
        assert!(
            wire.chunks.len() == 1,
            "nothing is read after a failed write"
        );
    }

    #[test]
    fn exchange_breaks_on_an_overlong_reply_line() {
        let mut line = vec![b'a'; 4097];
        line.push(b'\n');
        let mut wire = FakeWire::replying(vec![Ok(line)]);
        assert_eq!(exchange(&mut wire, &ping(7), WAIT), Err(SendError::Broken));
    }

    #[test]
    fn exchange_returns_a_dll_error_reply_for_the_caller_to_map() {
        let mut wire =
            FakeWire::line("{\"id\":7,\"ok\":false,\"code\":\"no_client\",\"detail\":\"x\"}\n");
        assert_eq!(
            exchange(&mut wire, &ping(7), WAIT),
            Ok(Reply::Error {
                id: 7,
                code: ErrorCode::NoClient,
                detail: "x".into()
            })
        );
    }

    #[derive(Default)]
    struct FakeListener {
        script: VecDeque<Result<Option<u32>, SendError>>,
        disconnects: u32,
    }

    impl Listener for FakeListener {
        fn wait_client(&mut self, _wait: Duration) -> Result<Option<u32>, SendError> {
            self.script.pop_front().unwrap_or(Ok(None))
        }
        fn disconnect(&mut self) {
            self.disconnects += 1;
        }
    }

    fn listener(script: Vec<Result<Option<u32>, SendError>>) -> FakeListener {
        FakeListener {
            script: script.into(),
            disconnects: 0,
        }
    }

    #[test]
    fn accept_game_takes_the_game_and_drops_other_clients() {
        let mut l = listener(vec![Ok(Some(42))]);
        assert_eq!(accept_game(&mut l, 42, CONNECT_WAIT), Ok(()));
        assert_eq!(l.disconnects, 0);

        let mut l = listener(vec![Ok(Some(99)), Ok(Some(42))]);
        assert_eq!(accept_game(&mut l, 42, CONNECT_WAIT), Ok(()));
        assert_eq!(l.disconnects, 1);
    }

    #[test]
    fn accept_game_gives_up_when_only_wrong_clients_come() {
        let mut l = listener(vec![Ok(Some(99)), Ok(Some(98))]);
        assert_eq!(
            accept_game(&mut l, 42, CONNECT_WAIT),
            Err(SendError::NoConnect)
        );
        assert_eq!(l.disconnects, 2);
    }

    #[test]
    fn accept_game_passes_a_wait_error_on() {
        let mut l = listener(vec![Err(SendError::PipeCreate)]);
        assert_eq!(
            accept_game(&mut l, 42, CONNECT_WAIT),
            Err(SendError::PipeCreate)
        );
    }

    struct FakeHost {
        calls: Vec<String>,
        create: Result<(), SendError>,
        loaded: VecDeque<bool>,
        gone_after_wait: bool,
        verify: VecDeque<Result<(), SendError>>,
        load: Result<(), SendError>,
        accept: Result<(), SendError>,
        hellos: VecDeque<Result<Reply, SendError>>,
        bye_gone: bool,
    }

    impl FakeHost {
        fn happy() -> Self {
            FakeHost {
                calls: Vec::new(),
                create: Ok(()),
                loaded: VecDeque::new(),
                gone_after_wait: true,
                verify: VecDeque::new(),
                load: Ok(()),
                accept: Ok(()),
                hellos: VecDeque::from([Ok(good_hello())]),
                bye_gone: true,
            }
        }
        fn count(&self, name: &str) -> usize {
            self.calls.iter().filter(|c| c.starts_with(name)).count()
        }
    }

    fn good_hello() -> Reply {
        Reply::Hello {
            id: 1,
            version: PIPE_PROTOCOL_VERSION,
            dll: "0.1.0".into(),
        }
    }

    impl Host for FakeHost {
        fn create_pipe(&mut self, pid: u32) -> Result<(), SendError> {
            self.calls.push(format!("create_pipe({pid})"));
            self.create.clone()
        }
        fn module_loaded(&mut self, _pid: u32) -> bool {
            self.calls.push("module_loaded".into());
            self.loaded.pop_front().unwrap_or(false)
        }
        fn wait_module_gone(&mut self, _pid: u32, wait: Duration) -> bool {
            assert_eq!(wait, UNLOAD_WAIT);
            self.calls.push("wait_module_gone".into());
            self.gone_after_wait
        }
        fn verify_dll(&mut self) -> Result<(), SendError> {
            self.calls.push("verify_dll".into());
            self.verify.pop_front().unwrap_or(Ok(()))
        }
        fn load_dll(&mut self, pid: u32) -> Result<(), SendError> {
            self.calls.push(format!("load_dll({pid})"));
            self.load.clone()
        }
        fn accept(&mut self, _pid: u32) -> Result<(), SendError> {
            self.calls.push("accept".into());
            self.accept.clone()
        }
        fn hello(&mut self) -> Result<Reply, SendError> {
            self.calls.push("hello".into());
            self.hellos.pop_front().expect("scripted hello")
        }
        fn bye_and_wait(&mut self, _pid: u32) -> bool {
            self.calls.push("bye_and_wait".into());
            self.bye_gone
        }
    }

    const RESIDENT_SENTENCE: &str =
        "게임에 전송 모듈이 남아 있어 다시 불러올 수 없습니다. 게임을 다시 시작해 주세요.";

    #[test]
    fn connect_runs_the_load_sequence_in_order_with_the_pipe_first() {
        let mut host = FakeHost::happy();
        assert_eq!(connect(&mut host, &mut Resident::default(), 42), Ok(42));
        assert_eq!(
            host.calls,
            [
                "create_pipe(42)",
                "module_loaded",
                "verify_dll",
                "load_dll(42)",
                "accept",
                "hello"
            ]
        );
    }

    #[test]
    fn connect_stops_when_the_pipe_cannot_be_created() {
        let mut host = FakeHost::happy();
        host.create = Err(SendError::OpenFailed);
        assert_eq!(
            connect(&mut host, &mut Resident::default(), 42),
            Err(SendError::PipeCreate)
        );
        assert_eq!(host.count("load_dll"), 0);
    }

    #[test]
    fn connect_for_another_pid_is_the_normal_sequence_beside_a_resident_one() {
        let mut resident = Resident::new();
        resident.note(42);
        let mut host = FakeHost::happy();
        assert_eq!(connect(&mut host, &mut resident, 7), Ok(7));
        assert_eq!(
            host.calls,
            [
                "create_pipe(7)",
                "module_loaded",
                "verify_dll",
                "load_dll(7)",
                "accept",
                "hello"
            ]
        );
        assert!(resident.holds(42));
    }

    #[test]
    fn connect_waits_for_an_old_module_to_leave_or_reports_it_stale() {
        let mut host = FakeHost::happy();
        host.loaded = VecDeque::from([true]);
        assert_eq!(connect(&mut host, &mut Resident::default(), 42), Ok(42));
        assert_eq!(host.count("wait_module_gone"), 1);

        let mut host = FakeHost::happy();
        host.loaded = VecDeque::from([true]);
        host.gone_after_wait = false;
        assert_eq!(
            connect(&mut host, &mut Resident::default(), 42),
            Err(SendError::ResidentModule)
        );
        assert_eq!(host.count("load_dll"), 0);
    }

    #[test]
    fn connect_does_not_load_a_file_that_fails_the_pin() {
        for err in [
            SendError::NoPin,
            SendError::DllMissing,
            SendError::DllDownloading,
            SendError::PinMismatch,
        ] {
            let mut host = FakeHost::happy();
            host.verify = VecDeque::from([Err(err.clone())]);
            assert_eq!(connect(&mut host, &mut Resident::default(), 42), Err(err));
            assert_eq!(host.count("load_dll"), 0);
        }
    }

    #[test]
    fn connect_passes_load_accept_and_hello_errors_on() {
        let mut host = FakeHost::happy();
        host.load = Err(SendError::LoadFailed);
        assert_eq!(
            connect(&mut host, &mut Resident::default(), 42),
            Err(SendError::LoadFailed)
        );
        assert_eq!(host.count("accept"), 0);

        let mut host = FakeHost::happy();
        host.accept = Err(SendError::NoConnect);
        assert_eq!(
            connect(&mut host, &mut Resident::default(), 42),
            Err(SendError::NoConnect)
        );

        let mut host = FakeHost::happy();
        host.hellos = VecDeque::from([Err(SendError::Broken)]);
        assert_eq!(
            connect(&mut host, &mut Resident::default(), 42),
            Err(SendError::Broken)
        );
    }

    fn version_error() -> Reply {
        Reply::Error {
            id: 1,
            code: ErrorCode::Version,
            detail: String::new(),
        }
    }

    #[test]
    fn connect_replaces_a_stale_module_once() {
        let mut host = FakeHost::happy();
        host.hellos = VecDeque::from([Ok(version_error()), Ok(good_hello())]);
        assert_eq!(connect(&mut host, &mut Resident::default(), 42), Ok(42));
        assert_eq!(host.count("bye_and_wait"), 1);
        assert_eq!(host.count("load_dll"), 2);
    }

    #[test]
    fn a_module_that_stays_after_bye_is_resident_and_is_not_loaded_again() {
        let mut host = FakeHost::happy();
        host.hellos = VecDeque::from([Ok(version_error())]);
        host.bye_gone = false;
        let mut resident = Resident::default();
        assert_eq!(
            connect(&mut host, &mut resident, 42),
            Err(SendError::ResidentModule)
        );
        assert_eq!(host.count("bye_and_wait"), 1);
        assert_eq!(host.count("load_dll"), 1);
        assert_eq!(host.count("accept"), 1);
        assert!(resident.holds(42));
    }

    #[test]
    fn a_module_that_stays_at_the_start_is_remembered() {
        let mut host = FakeHost::happy();
        host.loaded = VecDeque::from([true]);
        host.gone_after_wait = false;
        let mut resident = Resident::default();
        assert_eq!(
            connect(&mut host, &mut resident, 42),
            Err(SendError::ResidentModule)
        );
        assert_eq!(host.count("load_dll"), 0);
        assert_eq!(host.count("accept"), 0);
        assert!(resident.holds(42));
    }

    #[test]
    fn the_memory_fails_at_once_while_the_module_is_there() {
        let mut resident = Resident::default();
        resident.note(42);
        let mut host = FakeHost::happy();
        host.loaded = VecDeque::from([true]);
        assert_eq!(
            connect(&mut host, &mut resident, 42),
            Err(SendError::ResidentModule)
        );
        assert_eq!(host.calls, ["module_loaded"]);
    }

    #[test]
    fn the_memory_is_cleared_when_the_module_is_gone_or_the_game_is_another() {
        let mut resident = Resident::default();
        resident.note(42);
        let mut host = FakeHost::happy();
        host.loaded = VecDeque::from([false, false]);
        assert_eq!(connect(&mut host, &mut resident, 42), Ok(42));
        assert!(!resident.holds(42));
        assert_eq!(host.count("create_pipe"), 1);

        let mut resident = Resident::default();
        resident.note(7);
        let mut host = FakeHost::happy();
        assert_eq!(connect(&mut host, &mut resident, 42), Ok(42));
        assert_eq!(host.count("create_pipe"), 1);
        assert_eq!(host.count("load_dll"), 1);
    }

    #[test]
    fn the_resident_sentence_is_one_line_and_its_own() {
        let text = SendError::ResidentModule.to_string();
        assert_eq!(text, RESIDENT_SENTENCE);
        assert!(!text.contains('\n'));
        for other in [
            SendError::NoConnect,
            SendError::Broken,
            SendError::LoadFailed,
        ] {
            assert_ne!(other.to_string(), text);
        }
    }

    #[test]
    fn connect_gives_up_after_two_version_mismatches() {
        let mut host = FakeHost::happy();
        host.hellos = VecDeque::from([Ok(version_error()), Ok(version_error())]);
        assert_eq!(
            connect(&mut host, &mut Resident::default(), 42),
            Err(SendError::Dll(ErrorCode::Version))
        );
        assert_eq!(host.count("bye_and_wait"), 1);

        let mut host = FakeHost::happy();
        let other = Reply::Hello {
            id: 1,
            version: 2,
            dll: "x".into(),
        };
        host.hellos = VecDeque::from([Ok(other.clone()), Ok(other)]);
        assert_eq!(
            connect(&mut host, &mut Resident::default(), 42),
            Err(SendError::Dll(ErrorCode::Version))
        );
    }

    #[derive(Default)]
    struct FakeBackend {
        calls: Vec<(SendChatChannel, String)>,
        targets: Vec<Option<u32>>,
        result: Option<SendError>,
        sleep: Duration,
    }

    impl SendBackend for FakeBackend {
        fn send(
            &mut self,
            target: Option<u32>,
            channel: SendChatChannel,
            text: &str,
        ) -> Result<(), SendError> {
            self.targets.push(target);
            self.calls.push((channel, text.to_string()));
            std::thread::sleep(self.sleep);
            match self.result.clone() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
    }

    struct Shared(Arc<Mutex<FakeBackend>>);

    impl SendBackend for Shared {
        fn send(
            &mut self,
            target: Option<u32>,
            channel: SendChatChannel,
            text: &str,
        ) -> Result<(), SendError> {
            self.0.lock().unwrap().send(target, channel, text)
        }
    }

    fn too_soon_sentence() -> String {
        SendTextError::TooSoon { wait_ms: 1 }
            .to_string()
            .chars()
            .take(8)
            .collect()
    }

    #[test]
    fn the_flow_spaces_each_client_on_its_own() {
        let spacing = ClientSpacing::default();
        let mut backend = FakeBackend::default();
        let mut go = |backend: &mut FakeBackend, t| {
            send_chat_flow(true, &spacing, backend, t, SendChatChannel::Say, "hi")
        };
        assert_eq!(go(&mut backend, Some(7)), Ok(()));
        assert_eq!(go(&mut backend, Some(42)), Ok(()));
        assert_eq!(backend.targets, [Some(7), Some(42)]);
        let got = go(&mut backend, Some(7)).unwrap_err();
        assert!(got.starts_with(&too_soon_sentence()));
        assert_eq!(backend.targets.len(), 2);

        let spacing_none = ClientSpacing::default();
        let mut backend = FakeBackend::default();
        let mut go = |t| {
            send_chat_flow(
                true,
                &spacing_none,
                &mut backend,
                t,
                SendChatChannel::Say,
                "hi",
            )
        };
        assert_eq!(go(None), Ok(()));
        assert!(go(None).is_err());
    }

    #[test]
    fn a_failed_send_to_a_client_does_not_start_its_window() {
        let spacing = ClientSpacing::default();
        let mut backend = FakeBackend {
            result: Some(SendError::ClientGone),
            ..Default::default()
        };
        let go = |b: &mut FakeBackend| {
            send_chat_flow(true, &spacing, b, Some(7), SendChatChannel::Say, "hi")
        };
        assert!(go(&mut backend).is_err());
        backend.result = None;
        assert_eq!(go(&mut backend), Ok(()));
    }

    #[test]
    fn the_flow_refuses_when_the_setting_is_off_before_anything_else() {
        let spacing = Mutex::new(SendSpacing::default());
        let mut backend = FakeBackend::default();
        let got = send_chat_flow(
            false,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Say,
            "hi",
        );
        assert_eq!(got, Err(SendError::SettingOff.to_string()));
        assert!(backend.calls.is_empty());
    }

    #[test]
    fn the_flow_passes_the_target_to_the_backend() {
        let spacing = Mutex::new(SendSpacing::default());
        let mut backend = FakeBackend::default();
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            Some(7),
            SendChatChannel::Say,
            "hi",
        );
        assert_eq!(got, Ok(()));
        let spacing = Mutex::new(SendSpacing::default());
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Say,
            "hi",
        );
        assert_eq!(got, Ok(()));
        assert_eq!(backend.targets, [Some(7), None]);
    }

    #[test]
    fn the_flow_checks_the_text_before_the_backend() {
        let spacing = Mutex::new(SendSpacing::default());
        let mut backend = FakeBackend::default();
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Say,
            "   ",
        );
        assert_eq!(got, Err("보낼 내용이 없습니다.".to_string()));
        let long = "a".repeat(141);
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Say,
            &long,
        );
        assert_eq!(got, Err(SendTextError::TooLong { len: 141 }.to_string()));
        assert!(backend.calls.is_empty());
    }

    #[test]
    fn the_flow_sends_the_prepared_text_and_records_only_a_success() {
        let spacing = Mutex::new(SendSpacing::default());
        let mut backend = FakeBackend::default();
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Guild,
            "  hi\n",
        );
        assert_eq!(got, Ok(()));
        assert_eq!(backend.calls, [(SendChatChannel::Guild, "hi".to_string())]);

        let again = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Guild,
            "hi",
        );
        assert!(again.unwrap_err().starts_with(&too_soon_sentence()));
        assert_eq!(backend.calls.len(), 1);
    }

    #[test]
    fn a_failed_send_does_not_start_the_spacing_window() {
        let spacing = Mutex::new(SendSpacing::default());
        let mut backend = FakeBackend {
            result: Some(SendError::Dll(ErrorCode::NoClient)),
            ..Default::default()
        };
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Say,
            "hi",
        );
        assert_eq!(got, Err(code_text(ErrorCode::NoClient).to_string()));
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Say,
            "hi",
        );
        assert_eq!(got, Err(code_text(ErrorCode::NoClient).to_string()));
        assert_eq!(backend.calls.len(), 2);
    }

    #[test]
    fn two_sends_at_once_let_exactly_one_through() {
        let spacing = Mutex::new(SendSpacing::default());
        let backend = Arc::new(Mutex::new(FakeBackend {
            sleep: Duration::from_millis(50),
            ..Default::default()
        }));
        let results: Vec<Result<(), String>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    s.spawn(|| {
                        let mut shared = Shared(backend.clone());
                        send_chat_flow(
                            true,
                            &spacing,
                            &mut shared,
                            None,
                            SendChatChannel::Say,
                            "hi",
                        )
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        let err = results.into_iter().find_map(Result::err).unwrap();
        assert!(err.starts_with(&too_soon_sentence()));
        assert_eq!(backend.lock().unwrap().calls.len(), 1);
    }

    #[test]
    fn a_poisoned_spacing_mutex_still_works() {
        let spacing = Mutex::new(SendSpacing::default());
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _guard = spacing.lock().unwrap();
                panic!("poison");
            })
            .join()
        });
        assert!(spacing.is_poisoned());
        let mut backend = FakeBackend::default();
        let got = send_chat_flow(
            true,
            &spacing,
            &mut backend,
            None,
            SendChatChannel::Say,
            "hi",
        );
        assert_eq!(got, Ok(()));
        let _ = Instant::now();
    }

    #[test]
    fn a_send_reply_maps_to_a_result_and_whether_to_drop_the_session() {
        let error = |code| Reply::Error {
            id: 1,
            code,
            detail: "d".into(),
        };
        assert_eq!(
            map_send_reply(Reply::Send { id: 1, call_id: 9 }),
            (Ok(()), false)
        );
        assert_eq!(map_send_reply(Reply::Ack { id: 1 }), (Ok(()), false));
        assert_eq!(
            map_send_reply(error(ErrorCode::NoClient)),
            (Err(SendError::Dll(ErrorCode::NoClient)), false)
        );
        for code in [ErrorCode::BadRequest, ErrorCode::Version] {
            assert_eq!(
                map_send_reply(error(code)),
                (Err(SendError::Dll(code)), true)
            );
        }
        let hello = Reply::Hello {
            id: 1,
            version: 1,
            dll: String::new(),
        };
        assert_eq!(map_send_reply(hello), (Err(SendError::Broken), true));
    }
}
