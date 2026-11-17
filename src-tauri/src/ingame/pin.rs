//! The SHA-256 of the DLL this app was built with, written by `build.rs`.

include!(concat!(env!("OUT_DIR"), "/ingame_pin.rs"));

/// The file to load instead of the downloaded module of this version,
/// `<app data>\bin\ingame\<version>\resonance_ingame.dll`: the build-time path,
/// in a debug build only (a dev convenience).
pub fn dll_path_override() -> Option<&'static str> {
    if cfg!(debug_assertions) {
        INGAME_DLL_BUILD_PATH
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_core::ingame_send::{check_pin, PinError};

    #[test]
    fn the_pin_is_none_or_64_lower_case_hex_digits() {
        if let Some(pin) = INGAME_DLL_SHA256 {
            assert_eq!(pin.len(), 64);
            assert!(pin
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        }
    }

    #[test]
    fn without_a_pin_nothing_passes_the_check() {
        assert_eq!(check_pin(None, &"0".repeat(64)), Err(PinError::NoPin));
    }
}
