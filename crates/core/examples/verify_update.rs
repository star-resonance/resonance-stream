//! Checks an update exe the way the app will before it installs it: signed by
//! one of the keys built into the app, for the given version.
//!
//! `candidate.yml` and `promote.yml` run it on the signed exe, so a release the app would
//! refuse (wrong private key in the secret, wrong version) is never published.
//!
//!     cargo run --release -p resonance-core --example verify_update -- \
//!         <exe> <exe>.sig <version>

use resonance_core::update_signature::verify_with_app_keys;
use std::process::ExitCode;
use std::{env, fs};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let [exe, sig, version] = args.as_slice() else {
        eprintln!("usage: verify_update <exe> <exe.sig> <version>");
        return ExitCode::from(2);
    };
    let data = match fs::read(exe) {
        Ok(data) => data,
        Err(e) => return fail(format!("cannot read {exe}: {e}")),
    };
    let signature = match fs::read_to_string(sig) {
        Ok(text) => text,
        Err(e) => return fail(format!("cannot read {sig}: {e}")),
    };
    match verify_with_app_keys(&data, &signature, version) {
        Ok(()) => {
            println!("ok: {exe} is signed by an app key for version {version}");
            ExitCode::SUCCESS
        }
        Err(e) => fail(e.to_string()),
    }
}

fn fail(message: String) -> ExitCode {
    eprintln!("verify_update: {message}");
    ExitCode::FAILURE
}
