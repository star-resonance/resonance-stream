//! Raw IPv4/TCP packet → chat-message pipeline (reassembly, parsing, dedup, blocking).

mod message_processor;
mod pipeline;
mod recorder;
pub mod sanitise;
mod stream_tracker;
pub mod synth;

pub use self::pipeline::{ChatPipeline, PipelineAction};
pub use self::recorder::{
    decode_line, encode_line, is_capturable, replay, RawCaptureWriter, Recorded, DEFAULT_MAX_BYTES,
};
