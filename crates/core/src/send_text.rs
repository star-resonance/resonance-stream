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
            Err(SendTextError::TooSoon { wait_ms: 1000 })
        );
    }

    #[test]
    fn spacing_is_1500_ms() {
        assert_eq!(SEND_SPACING, Duration::from_millis(1500));
    }
}
