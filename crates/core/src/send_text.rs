//! Spacing between chat sends. The text rules and the error type are in
//! `resonance-types` (`prepare_text`, `SendTextError`) so the ui can use them.

use resonance_types::SendTextError;
use std::time::{Duration, Instant};

/// A send less than this long after the last successful one is refused.
pub const SEND_SPACING: Duration = Duration::from_millis(1500);

/// Remembers when the last send succeeded. Unlike `paste::RepeatGuard`,
/// [`SendSpacing::allow`] does not record: only a send that went through counts,
/// so the caller calls [`SendSpacing::record_success`] after it.
#[derive(Debug, Default)]
pub struct SendSpacing {
    last_ok: Option<Instant>,
}

impl SendSpacing {
    /// Ok when a send may go out at `now`; otherwise `TooSoon` with the whole
    /// milliseconds left (rounded up, at least 1). Never moves the window.
    ///
    /// Check and record are separate steps: a caller that can run twice at once
    /// holds one lock across `allow`, the send and `record_success`, or two sends
    /// can both pass.
    pub fn allow(&self, now: Instant) -> Result<(), SendTextError> {
        let Some(last) = self.last_ok else {
            return Ok(());
        };
        let elapsed = now.saturating_duration_since(last);
        if elapsed >= SEND_SPACING {
            return Ok(());
        }
        let left = SEND_SPACING - elapsed;
        let wait_ms = left.as_micros().div_ceil(1000).max(1) as u64;
        Err(SendTextError::TooSoon { wait_ms })
    }

    /// Records that a send succeeded at `now`.
    pub fn record_success(&mut self, now: Instant) {
        self.last_ok = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_new_gate_allows() {
        assert_eq!(SendSpacing::default().allow(Instant::now()), Ok(()));
    }

    #[test]
    fn refuses_within_the_spacing_with_the_time_left() {
        let mut gate = SendSpacing::default();
        let t0 = Instant::now();
        gate.record_success(t0);
        assert_eq!(
            gate.allow(t0 + ms(300)),
            Err(SendTextError::TooSoon { wait_ms: 1200 })
        );
        assert_eq!(
            gate.allow(t0 + ms(1499)),
            Err(SendTextError::TooSoon { wait_ms: 1 })
        );
        assert_eq!(gate.allow(t0 + ms(1500)), Ok(()));
        assert_eq!(
            gate.allow(t0),
            Err(SendTextError::TooSoon { wait_ms: 1500 })
        );
    }

    #[test]
    fn a_refused_allow_does_not_move_the_window() {
        let mut gate = SendSpacing::default();
        let t0 = Instant::now();
        gate.record_success(t0);
        assert!(gate.allow(t0 + ms(300)).is_err());
        assert_eq!(gate.allow(t0 + ms(1500)), Ok(()));
    }

    #[test]
    fn allow_never_records() {
        let gate = SendSpacing::default();
        let t0 = Instant::now();
        assert_eq!(gate.allow(t0), Ok(()));
        assert_eq!(gate.allow(t0), Ok(()));
    }

    #[test]
    fn a_later_success_moves_the_window() {
        let mut gate = SendSpacing::default();
        let t0 = Instant::now();
        gate.record_success(t0);
        gate.record_success(t0 + ms(2000));
        assert_eq!(
            gate.allow(t0 + ms(3000)),
            Err(SendTextError::TooSoon { wait_ms: 500 })
        );
    }

    #[test]
    fn a_clients_window_never_delays_another_client() {
        let spacing = ClientSpacing::default();
        let (a, b) = (Some(7), Some(42));
        let t0 = Instant::now();
        spacing.record_success(a, t0);
        assert_eq!(
            spacing.allow(a, t0 + ms(300)),
            Err(SendTextError::TooSoon { wait_ms: 1200 })
        );
        assert_eq!(spacing.allow(b, t0 + ms(300)), Ok(()));
        assert_eq!(spacing.allow(None, t0 + ms(300)), Ok(()));
        assert_eq!(spacing.allow(a, t0 + ms(1500)), Ok(()));
    }

    #[test]
    fn one_key_keeps_the_single_window_rules() {
        let spacing = ClientSpacing::default();
        let t0 = Instant::now();
        assert_eq!(spacing.allow(None, t0), Ok(()));
        assert_eq!(spacing.allow(None, t0), Ok(()));
        spacing.record_success(None, t0);
        assert_eq!(
            spacing.allow(None, t0 + ms(1499)),
            Err(SendTextError::TooSoon { wait_ms: 1 })
        );
        assert_eq!(
            spacing.allow(None, t0 + ms(300)),
            Err(SendTextError::TooSoon { wait_ms: 1200 })
        );
        spacing.record_success(None, t0 + ms(2000));
        assert_eq!(
            spacing.allow(None, t0 + ms(3000)),
            Err(SendTextError::TooSoon { wait_ms: 500 })
        );
    }

    #[test]
    fn an_idle_slot_is_dropped_when_another_is_fetched() {
        let spacing = ClientSpacing::default();
        let t0 = Instant::now();
        spacing.record_success(Some(7), t0);
        assert_eq!(spacing.len(), 1);
        let _b = spacing.slot(Some(42), t0 + ms(2000));
        assert_eq!(spacing.len(), 1);
    }

    #[test]
    fn a_slot_held_by_a_running_send_is_kept() {
        let spacing = ClientSpacing::default();
        let t0 = Instant::now();
        let a = spacing.slot(Some(7), t0);
        let _b = spacing.slot(Some(42), t0 + ms(2000));
        assert_eq!(spacing.len(), 2);
        drop(a);
    }

    #[test]
    fn spacing_is_1500_ms() {
        assert_eq!(SEND_SPACING, Duration::from_millis(1500));
    }
}
