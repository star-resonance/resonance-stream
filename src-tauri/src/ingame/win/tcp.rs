//! Which processes hold a connection to the game's chat port.

use resonance_core::game_clients::{is_established, row_endpoint, Endpoint};
use resonance_core::ingame_send::{remote_port, CHAT_PORT};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
};
use windows_sys::Win32::Networking::WinSock::AF_INET;

const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

/// One row of the IPv4 TCP table.
struct Row {
    pid: u32,
    state: u32,
    local_addr: u32,
    local_port: u32,
    remote_port: u32,
}

/// The whole IPv4 TCP table, read once (empty when the call fails).
fn table_rows() -> Vec<Row> {
    // 8-byte aligned storage for the table.
    let mut storage: Vec<u64> = vec![0; 1024];
    for _ in 0..4 {
        let mut size = (storage.len() * 8) as u32;
        // SAFETY: the buffer is `size` bytes; the table is written into it.
        let result = unsafe {
            GetExtendedTcpTable(
                storage.as_mut_ptr().cast(),
                &mut size,
                0,
                AF_INET as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if result == ERROR_INSUFFICIENT_BUFFER {
            storage = vec![0; (size as usize).div_ceil(8) + 64];
            continue;
        }
        if result != 0 {
            return Vec::new();
        }
        let table = storage.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        // SAFETY: on success the buffer starts with a MIB_TCPTABLE_OWNER_PID whose
        // `dwNumEntries` rows follow it in the same buffer.
        let (count, rows) = unsafe {
            (
                (*table).dwNumEntries as usize,
                std::ptr::addr_of!((*table).table).cast::<MIB_TCPROW_OWNER_PID>(),
            )
        };
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            // SAFETY: `i < dwNumEntries`, so the row is inside the buffer.
            let row = unsafe { &*rows.add(i) };
            out.push(Row {
                pid: row.dwOwningPid,
                state: row.dwState,
                local_addr: row.dwLocalAddr,
                local_port: row.dwLocalPort,
                remote_port: row.dwRemotePort,
            });
        }
        return out;
    }
    Vec::new()
}

/// The pids that own an IPv4 TCP connection whose remote port is [`CHAT_PORT`].
pub fn chat_port_owners() -> Vec<u32> {
    let mut owners = Vec::new();
    for row in table_rows() {
        if remote_port(row.remote_port) == CHAT_PORT && !owners.contains(&row.pid) {
            owners.push(row.pid);
        }
    }
    owners
}

/// The established connections to [`CHAT_PORT`]: the owning pid and the local end of each.
pub fn chat_connections() -> Vec<(u32, Endpoint)> {
    table_rows()
        .into_iter()
        .filter(|r| remote_port(r.remote_port) == CHAT_PORT && is_established(r.state))
        .map(|r| (r.pid, row_endpoint(r.local_addr, r.local_port)))
        .collect()
}
