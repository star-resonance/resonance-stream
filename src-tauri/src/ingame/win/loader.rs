//! Modules of the game process, and the remote `LoadLibraryW` thread.

use super::Handle;
use resonance_core::ingame_send::{SendError, DLL_NAME, LOAD_WAIT};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W, TH32CS_SNAPMODULE,
    TH32CS_SNAPMODULE32,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject, PROCESS_CREATE_THREAD,
    PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

/// True when a module named [`DLL_NAME`] is in the process `pid`.
pub fn module_loaded(pid: u32) -> bool {
    // SAFETY: a plain snapshot call; the handle is owned and closed by `Handle`.
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    let Some(snapshot) = Handle::new(snapshot) else {
        return false;
    };
    // SAFETY: MODULEENTRY32W is plain data; dwSize is set before the first call.
    let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
    // SAFETY: `snapshot` is live and `entry` is a valid, sized struct.
    let mut more = unsafe { Module32FirstW(snapshot.raw(), &mut entry) } != 0;
    while more {
        let len = entry
            .szModule
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szModule.len());
        if String::from_utf16_lossy(&entry.szModule[..len]).eq_ignore_ascii_case(DLL_NAME) {
            return true;
        }
        // SAFETY: as above.
        more = unsafe { Module32NextW(snapshot.raw(), &mut entry) } != 0;
    }
    false
}

/// Polls until the module has left `pid` or `wait` has passed; true once it has.
pub fn wait_module_gone(pid: u32, wait: Duration) -> bool {
    let started = Instant::now();
    loop {
        if !module_loaded(pid) {
            return true;
        }
        if started.elapsed() >= wait {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Memory in another process, freed on drop.
struct RemoteMemory<'a> {
    process: &'a Handle,
    address: *mut core::ffi::c_void,
}

impl Drop for RemoteMemory<'_> {
    fn drop(&mut self) {
        // SAFETY: `address` came from VirtualAllocEx on `process` and is freed once.
        unsafe { VirtualFreeEx(self.process.raw(), self.address, 0, MEM_RELEASE) };
    }
}

/// Loads `dll` into the process `pid` with a remote thread running `LoadLibraryW`,
/// and waits up to [`LOAD_WAIT`] for it. The thread's exit code is a truncated
/// `HMODULE`, so only zero (failed) and non-zero (loaded) are read.
pub fn load_remote(pid: u32, dll: &Path) -> Result<(), SendError> {
    let access = PROCESS_CREATE_THREAD
        | PROCESS_QUERY_INFORMATION
        | PROCESS_VM_OPERATION
        | PROCESS_VM_WRITE
        | PROCESS_VM_READ;
    // SAFETY: a plain OpenProcess; the handle is owned and closed by `Handle`.
    let process =
        Handle::new(unsafe { OpenProcess(access, 0, pid) }).ok_or(SendError::OpenFailed)?;

    let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(Some(0)).collect();
    let bytes = wide.len() * 2;
    // SAFETY: allocates `bytes` of read/write memory in `process`.
    let address = unsafe {
        VirtualAllocEx(
            process.raw(),
            std::ptr::null(),
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if address.is_null() {
        return Err(SendError::LoadFailed);
    }
    let remote = RemoteMemory {
        process: &process,
        address,
    };
    // SAFETY: `address` has room for `bytes`; `wide` is `bytes` long.
    let written = unsafe {
        WriteProcessMemory(
            process.raw(),
            remote.address,
            wide.as_ptr().cast(),
            bytes,
            std::ptr::null_mut(),
        )
    };
    if written == 0 {
        return Err(SendError::LoadFailed);
    }

    let kernel32: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
    // SAFETY: kernel32 is loaded at the same address in every process of a boot, so
    // the local address of LoadLibraryW is also valid in the game.
    let load_library = unsafe {
        let module = GetModuleHandleW(kernel32.as_ptr());
        if module.is_null() {
            return Err(SendError::LoadFailed);
        }
        GetProcAddress(module, c"LoadLibraryW".as_ptr().cast())
    };
    let Some(load_library) = load_library else {
        return Err(SendError::LoadFailed);
    };
    // SAFETY: LoadLibraryW takes one pointer argument and returns a pointer-sized
    // value, the shape of a thread start routine.
    let start: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32 =
        unsafe { std::mem::transmute(load_library) };
    // SAFETY: `start` is LoadLibraryW and its argument is the path written above.
    let thread = Handle::new(unsafe {
        CreateRemoteThread(
            process.raw(),
            std::ptr::null(),
            0,
            Some(start),
            remote.address,
            0,
            std::ptr::null_mut(),
        )
    })
    .ok_or(SendError::LoadFailed)?;

    // SAFETY: `thread` is a live thread handle.
    let waited = unsafe { WaitForSingleObject(thread.raw(), LOAD_WAIT.as_millis() as u32) };
    if waited != WAIT_OBJECT_0 {
        // The thread may still read the path: leave that memory in the game rather
        // than free it under the thread (a few hundred bytes).
        std::mem::forget(remote);
        return Err(SendError::LoadFailed);
    }
    let mut code: u32 = 0;
    // SAFETY: the thread has finished; `code` is valid for writes.
    let got = unsafe { GetExitCodeThread(thread.raw(), &mut code) };
    if got == 0 || code == 0 {
        return Err(SendError::LoadFailed);
    }
    Ok(())
}
