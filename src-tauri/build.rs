use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

/// Pins the in-game DLL the app ships: writes `$OUT_DIR/ingame_pin.rs` with the
/// file's SHA-256 (`INGAME_DLL_SHA256`) and the path it was read from
/// (`INGAME_DLL_BUILD_PATH`). `RESONANCE_INGAME_DLL` names the built DLL;
/// `RESONANCE_INGAME_REQUIRED` (not empty, not `0`) makes a missing file a build
/// error, so a Candidate or release can never ship without a pin. Without it a
/// missing file is a warning and the pin is `None` (the loader then refuses).
fn write_ingame_pin() {
    println!("cargo:rerun-if-env-changed=RESONANCE_INGAME_DLL");
    println!("cargo:rerun-if-env-changed=RESONANCE_INGAME_REQUIRED");
    let required = env::var("RESONANCE_INGAME_REQUIRED").is_ok_and(|v| !v.is_empty() && v != "0");
    let path = env::var("RESONANCE_INGAME_DLL")
        .ok()
        .filter(|p| !p.is_empty());
    let pinned = match path {
        Some(path) => {
            println!("cargo:rerun-if-changed={path}");
            match fs::read(&path) {
                Ok(bytes) => {
                    let hex: String = Sha256::digest(&bytes)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    Some((hex, path))
                }
                Err(e) if required => {
                    panic!("RESONANCE_INGAME_DLL={path} cannot be read: {e}")
                }
                Err(e) => {
                    println!("cargo:warning=RESONANCE_INGAME_DLL={path} cannot be read ({e}): no in-game DLL pin");
                    None
                }
            }
        }
        None if required => {
            panic!("RESONANCE_INGAME_REQUIRED is set but RESONANCE_INGAME_DLL is not")
        }
        None => {
            println!("cargo:warning=RESONANCE_INGAME_DLL is not set: no in-game DLL pin");
            None
        }
    };
    let source = match pinned {
        Some((hex, path)) => format!(
            "pub const INGAME_DLL_SHA256: Option<&str> = Some({hex:?});\n\
             pub const INGAME_DLL_BUILD_PATH: Option<&str> = Some({path:?});\n"
        ),
        None => "pub const INGAME_DLL_SHA256: Option<&str> = None;\n\
                 pub const INGAME_DLL_BUILD_PATH: Option<&str> = None;\n"
            .to_string(),
    };
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("ingame_pin.rs");
    fs::write(out, source).expect("write ingame_pin.rs");
}

fn main() {
    write_ingame_pin();
    // Standard Tauri build
    if cfg!(debug_assertions) {
        println!("DEV BUILD");
        tauri_build::build();
    } else {
        let mut windows = tauri_build::WindowsAttributes::new();
        windows = windows.app_manifest(include_str!("app.manifest"));

        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("failed to run build script");
    };
}
