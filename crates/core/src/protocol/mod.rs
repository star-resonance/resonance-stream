//! Port 5003 chat protocol: TCP stream framing and protobuf-style decoding.

pub(crate) mod compression;
pub(crate) mod decoder;
pub mod framing;
#[cfg(test)]
mod outbound;
pub mod own_player;
pub mod parser;
