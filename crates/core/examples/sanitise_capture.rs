//! Makes a raw capture (the app's debug "Raw Capture") safe to publish as a test corpus (S7):
//! nicknames, uids, chat text and addresses replaced, every packet's length and TCP sequence
//! number kept (see `resonance_core::capture::sanitise`).
//!
//!     cargo run -q --release -p resonance-core --example sanitise_capture -- <in> <out>
//!
//! Nothing is written unless the sanitised capture decodes to the original's lines under the
//! new names and none of the original's names, uids, lines or addresses is left in it. The
//! report on stdout is for the person who reads the result before it is committed.

use resonance_core::capture::sanitise::sanitise;
use std::path::Path;
use std::process::ExitCode;
use std::{env, fs};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: sanitise_capture <in.capture.log> <out>");
        return ExitCode::from(2);
    };
    if Path::new(output).exists() {
        return fail(format!("{output} exists; nothing is overwritten"));
    }
    let text = match fs::read_to_string(input) {
        Ok(text) => text,
        Err(e) => return fail(format!("cannot read {input}: {e}")),
    };
    let sanitised = match sanitise(&text) {
        Ok(sanitised) => sanitised,
        Err(e) => return fail(format!("{input}: {e}; nothing written")),
    };
    if let Err(e) = fs::write(output, &sanitised.text) {
        return fail(format!("cannot write {output}: {e}"));
    }
    let r = &sanitised.report;
    println!("wrote {output}");
    println!(
        "packets kept: {}, lines dropped: {}",
        r.packets, r.dropped_lines
    );
    println!(
        "chat lines the app shows: {} (the same before and after)",
        r.chats
    );
    println!(
        "replaced: {} nicknames, {} uids, {} addresses",
        r.nicknames, r.uids, r.addresses
    );
    let frames: Vec<String> = r
        .frames
        .iter()
        .map(|(ty, n)| format!("0x{ty:04x}: {n}"))
        .collect();
    println!("frames: {}", frames.join(", "));
    println!("frames with no chat, blanked: {}", r.frames_blanked);
    println!(
        "history frames compressed again and padded: {}",
        r.history_padded
    );
    println!("strings off the chat paths filled: {}", r.strings_filled);
    println!("uids replaced in packed lists: {}", r.packed_uids);
    println!("bytes blanked: {}", r.bytes_blanked);
    if r.binary_left.is_empty() {
        println!("byte fields left as they were: none");
    } else {
        println!("byte fields left as they were (check none can name anyone):");
        for line in &r.binary_left {
            println!("  {line}");
        }
    }
    if r.numbers_left.is_empty() {
        println!("numbers of 10 000 or more left as they were: none");
    } else {
        println!(
            "numbers of 10 000 or more left as they were (an id of someone who never spoke?):"
        );
        for line in &r.numbers_left {
            println!("  {line}");
        }
    }
    ExitCode::SUCCESS
}

fn fail(message: String) -> ExitCode {
    eprintln!("sanitise_capture: {message}");
    ExitCode::FAILURE
}
