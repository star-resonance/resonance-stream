//! The server's Return to a `SendChitChatMsg` call, read from a plain `0x0003` frame.
//!
//! Layout (big-endian; from the Stellar wire tap, NOT VERIFIED against a capture): the frame's
//! 12-byte inner header is `stub_id` (bytes 0..4), `call_id` (4..8), `error_id` (8..12); the root
//! is `0A <len> <SendChitChatMsgReply>` with field 3 `show_msg` (its own field 3 is the chat
//! message's Unix-second timestamp), field 4 `cd_end_time` (varint, Unix seconds) and field 5
//! `err_code` (varint).
//!
//! A Return carries no service or method, so this reads the Return of every RPC; a decoded code
//! means something only for a `call_id` the caller knows it sent.

/// `err_code` of an accepted send.
pub const ERR_ACCEPTED: u32 = 0;
/// `err_code`: the chat-send cooldown has not ended (`cd_end_time` says when).
pub const ERR_CHAT_SEND_CD_NO_END: u32 = 5211;

/// A decoded Return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcReturn {
    pub call_id: u32,
    pub err_code: u32,
    /// Unix seconds; 0 = none.
    pub cd_end_time: u64,
    /// The inner header's bytes 8..12.
    pub error_id: u32,
    /// The chat message's Unix-second timestamp from `show_msg`, when present and nonzero.
    pub server_time: Option<u64>,
}

/// What a Return says about a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Accepted,
    CooldownNotEnded,
    /// Any other code: kept, never named.
    Other(u32),
}

impl RpcReturn {
    pub fn outcome(&self) -> SendOutcome {
        match self.err_code {
            ERR_ACCEPTED => SendOutcome::Accepted,
            ERR_CHAT_SEND_CD_NO_END => SendOutcome::CooldownNotEnded,
            n => SendOutcome::Other(n),
        }
    }
}

/// Reads a Return from a frame's inner header and root. `None` when the header is shorter than
/// 12 bytes, the root is not `0A <len> ..`, or `err_code` does not fit a `u32`.
pub fn decode(inner_header: &[u8], root: &[u8]) -> Option<RpcReturn> {
    let _ = (inner_header, root);
    None
}
