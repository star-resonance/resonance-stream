//! Click-through with a hold key: while click-through is on, holding the
//! hold key lets the overlay take clicks until the key is let go. The app
//! polls the key (Windows); the decisions are here, testable on any OS.

use resonance_types::ClickThroughHoldKey;

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
