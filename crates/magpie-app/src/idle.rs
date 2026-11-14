//! Idle detection for time tracking (Phase 17 leftover). A pure policy fed by a
//! periodic sampler: when the user comes back after being idle for at least the
//! threshold while a timer was running, it reports the idle span **once**, so the
//! UI can ask "Keep or discard?" — it never edits time entries itself.

/// A stretch with no input while a timer ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleSpan {
    /// When input stopped.
    pub from_ms: i64,
    /// When input resumed.
    pub to_ms: i64,
}

impl IdleSpan {
    pub fn minutes(&self) -> i64 {
        (self.to_ms - self.from_ms) / 60_000
    }
}

/// Input within this window counts as "back" (the sampler runs every few seconds).
const BACK_MS: i64 = 60_000;

#[derive(Debug, Default)]
pub struct IdleWatch {
    /// Start of the current over-threshold idle stretch, if any.
    idle_since: Option<i64>,
}

impl IdleWatch {
    /// Feed one sample: `idle_secs` since the last input, whether a timer is running,
    /// and the threshold (`0` = off). Returns the span when the user returns.
    pub fn feed(
        &mut self,
        now_ms: i64,
        idle_secs: f64,
        timer_running: bool,
        threshold_min: i64,
    ) -> Option<IdleSpan> {
        if !timer_running || threshold_min <= 0 {
            self.idle_since = None;
            return None;
        }
        let idle_ms = (idle_secs * 1000.0) as i64;
        let last_input = now_ms - idle_ms;
        if idle_ms >= threshold_min * 60_000 {
            // Keep the earliest start if the stretch is already known.
            self.idle_since.get_or_insert(last_input);
            return None;
        }
        if idle_ms < BACK_MS {
            if let Some(from) = self.idle_since.take() {
                return Some(IdleSpan {
                    from_ms: from,
                    to_ms: last_input,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn reports_once_when_user_returns() {
        let mut w = IdleWatch::default();
        // Active, then idle for 12 min (threshold 10).
        assert_eq!(w.feed(0, 1.0, true, 10), None);
        assert_eq!(w.feed(12 * MIN, 12.0 * 60.0, true, 10), None);
        assert_eq!(w.feed(20 * MIN, 20.0 * 60.0, true, 10), None);
        // Input 3 s ago → back. Idle ran from minute 0 to 20 minus 3 s.
        let span = w.feed(20 * MIN + 3_000, 0.0 + 0.0, true, 10).unwrap();
        assert_eq!(span.from_ms, 0);
        assert_eq!(span.to_ms, 20 * MIN + 3_000);
        assert_eq!(span.minutes(), 20);
        // Only once.
        assert_eq!(w.feed(20 * MIN + 8_000, 1.0, true, 10), None);
    }

    #[test]
    fn short_breaks_off_and_stopped_timers_report_nothing() {
        let mut w = IdleWatch::default();
        assert_eq!(w.feed(5 * MIN, 5.0 * 60.0, true, 10), None); // under threshold
        assert_eq!(w.feed(5 * MIN + 1_000, 0.5, true, 10), None);
        // Threshold 0 = off.
        assert_eq!(w.feed(30 * MIN, 30.0 * 60.0, true, 0), None);
        assert_eq!(w.feed(30 * MIN + 1_000, 0.5, true, 0), None);
        // Timer stopped while idle → the stretch is dropped.
        assert_eq!(w.feed(50 * MIN, 15.0 * 60.0, true, 10), None);
        assert_eq!(w.feed(51 * MIN, 16.0 * 60.0, false, 10), None);
        assert_eq!(w.feed(51 * MIN + 1_000, 0.5, true, 10), None);
    }
}
