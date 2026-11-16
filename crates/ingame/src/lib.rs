//! `resonance_ingame.dll`: loaded into the game process by the app, it resolves the
//! game's IL2CPP API by name, probes the live `ZTcpClient` and answers the app's
//! named pipe. Send is not built yet.
//!
//! Build it only with `--profile ingame` (`panic = "unwind"`): a panic that aborts
//! would kill the game.
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(all(windows, panic = "abort"))]
compile_error!(
    "resonance-ingame must be built with `--profile ingame` (panic = \"unwind\"): \
     `cargo build -p resonance-ingame --profile ingame`"
);

pub mod error;
pub mod log;
pub mod names;
pub mod session;
pub mod unload;
pub mod walk;
