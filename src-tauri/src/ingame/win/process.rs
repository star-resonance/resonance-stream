//! The process list and the bitness of a process.

use super::Handle;
use resonance_core::ingame_send::ProcessEntry;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    IsWow64Process2, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Every running process (pid and exe file name). Empty when the snapshot fails.
pub fn list_processes() -> Vec<ProcessEntry> {
    // SAFETY: a plain snapshot call; the handle is owned and closed by `Handle`.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let Some(snapshot) = Handle::new(snapshot) else {
        return Vec::new();
    };
    // SAFETY: PROCESSENTRY32W is plain data; dwSize is set before the first call.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::new();
    // SAFETY: `snapshot` is a live snapshot handle and `entry` is a valid, sized struct.
    let mut more = unsafe { Process32FirstW(snapshot.raw(), &mut entry) } != 0;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        out.push(ProcessEntry {
            pid: entry.th32ProcessID,
            exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot.raw(), &mut entry) } != 0;
    }
    out
}

/// True when `pid` is a 32-bit process on 64-bit Windows. A process that cannot be
/// opened counts as not 32-bit (the load then fails with its own message).
pub fn is_32bit(pid: u32) -> bool {
    // SAFETY: a plain OpenProcess; the handle is owned and closed by `Handle`.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    let Some(process) = Handle::new(process) else {
        return false;
    };
    let mut process_machine: u16 = 0;
    let mut native_machine: u16 = 0;
    // SAFETY: `process` is a live handle; both out-pointers are valid for writes.
    let ok = unsafe { IsWow64Process2(process.raw(), &mut process_machine, &mut native_machine) };
    // IMAGE_FILE_MACHINE_UNKNOWN (0) means the process is not running under WOW64.
    ok != 0 && process_machine != 0
}
