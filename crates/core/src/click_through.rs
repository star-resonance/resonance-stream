//! Click-through with a hold key: while click-through is on, holding the
//! hold key lets the overlay take clicks until the key is let go. The app
//! polls the key (Windows); the decisions are here, testable on any OS.

use resonance_types::ClickThroughHoldKey;

/// Whether the window lets clicks through to the game: click-through is on
/// and the hold key is not down.
pub fn ignores_cursor(click_through: bool, hold_key_down: bool) -> bool {
    click_through && !hold_key_down
}

/// The Windows virtual-key code of the hold key (either side's key counts);
/// `None` for no key.
pub fn virtual_key(key: ClickThroughHoldKey) -> Option<u16> {
    match key {
        ClickThroughHoldKey::Shift => Some(0x10), // VK_SHIFT
        ClickThroughHoldKey::Ctrl => Some(0x11),  // VK_CONTROL
        ClickThroughHoldKey::Alt => Some(0x12),   // VK_MENU
        ClickThroughHoldKey::NoKey => None,
    }
}

/// The tray's click-through item: on or off, and the key that lifts it
/// while held.
pub fn tray_label(click_through: bool, key: ClickThroughHoldKey) -> String {
    let state = if click_through { "ON" } else { "OFF" };
    match key {
        ClickThroughHoldKey::NoKey => format!("클릭 관통 (Click-Through): {state}"),
        key => format!(
            "클릭 관통 (Click-Through): {state} · {} 누르는 동안 해제",
            key.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_takes_clicks_while_the_hold_key_is_down() {
        assert!(ignores_cursor(true, false));
        assert!(!ignores_cursor(true, true));
        // Click-through off: the window always takes clicks.
        assert!(!ignores_cursor(false, false));
        assert!(!ignores_cursor(false, true));
    }

    #[test]
    fn each_hold_key_is_its_windows_virtual_key() {
        assert_eq!(virtual_key(ClickThroughHoldKey::Shift), Some(0x10));
        assert_eq!(virtual_key(ClickThroughHoldKey::Ctrl), Some(0x11));
        assert_eq!(virtual_key(ClickThroughHoldKey::Alt), Some(0x12));
        assert_eq!(virtual_key(ClickThroughHoldKey::NoKey), None);
    }

    #[test]
    fn the_tray_item_names_the_hold_key() {
        assert_eq!(
            tray_label(true, ClickThroughHoldKey::Alt),
            "클릭 관통 (Click-Through): ON · Alt 누르는 동안 해제"
        );
        assert_eq!(
            tray_label(false, ClickThroughHoldKey::Ctrl),
            "클릭 관통 (Click-Through): OFF · Ctrl 누르는 동안 해제"
        );
        assert_eq!(
            tray_label(true, ClickThroughHoldKey::NoKey),
            "클릭 관통 (Click-Through): ON"
        );
    }
}
