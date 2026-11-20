//! Desktop reminders before the next prayer (Milestone 7).
//!
//! The decision logic (`should_notify`) is pure and unit-tested; the actual
//! D-Bus call sits behind the [`Notifier`] trait so tests can use a
//! capture-double. Notifications are opt-in (`notify: true` in config or the
//! `n` key in the TUI) and degrade silently on headless machines.

use anyhow::Result;

/// How many minutes before a prayer the reminder fires (default).
pub const DEFAULT_LEAD_MINUTES: u32 = 10;

/// Decide whether a reminder should be sent now.
///
/// - `now` — current unix epoch (seconds)
/// - `next_at` — epoch seconds of the upcoming prayer occurrence
/// - `lead_minutes` — reminder lead time
/// - `last_notified` — epoch of the prayer occurrence we already reminded for
///
/// Fires once per occurrence: starting the app inside the lead window still
/// notifies (better late than never); occurrences that have already passed
/// are never notified.
pub fn should_notify(now: i64, next_at: i64, lead_minutes: u32, last_notified: Option<i64>) -> bool {
    let lead_secs = i64::from(lead_minutes) * 60;
    let in_window = next_at > now && next_at - now <= lead_secs;
    in_window && last_notified != Some(next_at)
}

/// Abstraction over the desktop notification mechanism.
pub trait Notifier: Send {
    fn send(&self, title: &str, body: &str) -> Result<()>;
}

/// Real implementation: `org.freedesktop.Notifications` via `notify-rust`.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn send(&self, title: &str, body: &str) -> Result<()> {
        // Blocking D-Bus round-trip; callers run this in `spawn_blocking`.
        notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .icon("appointment")
            .timeout(notify_rust::Timeout::Milliseconds(10_000))
            .show()
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("desktop notification failed: {e}"))
    }
}

/// Capture-double for tests.
#[allow(dead_code)]
pub struct CapturingNotifier {
    pub sent: std::sync::Mutex<Vec<(String, String)>>,
}

impl CapturingNotifier {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self { sent: std::sync::Mutex::new(Vec::new()) }
    }
}

impl Default for CapturingNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier for CapturingNotifier {
    fn send(&self, title: &str, body: &str) -> Result<()> {
        self.sent.lock().unwrap().push((title.to_string(), body.to_string()));
        Ok(())
    }
}

/// Build the notification text for an upcoming prayer.
pub fn reminder_text(
    prayer: &str,
    prayer_at: i64,
    now: i64,
    zone_display: &str,
) -> (String, String) {
    let mins = ((prayer_at - now).max(0) / 60).max(1);
    let title = format!("Waktu Solat — {prayer} in {mins} minute{}", if mins == 1 { "" } else { "s" });
    let time = chrono::DateTime::from_timestamp(prayer_at, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default();
    let body = format!("{prayer} at {time} · {zone_display}");
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10-minute window around a prayer at T.
    const NEXT_AT: i64 = 1_800_000_000;

    #[test]
    fn fires_at_exact_boundary() {
        let now = NEXT_AT - 600;
        assert!(should_notify(now, NEXT_AT, 10, None));
    }

    #[test]
    fn fires_inside_window() {
        let now = NEXT_AT - 371;
        assert!(should_notify(now, NEXT_AT, 10, None));
    }

    #[test]
    fn silent_outside_window() {
        // Too early.
        assert!(!should_notify(NEXT_AT - 601, NEXT_AT, 10, None));
        // Already passed.
        assert!(!should_notify(NEXT_AT + 1, NEXT_AT, 10, None));
        // At the prayer time itself (diff = 0 counts as passed).
        assert!(!should_notify(NEXT_AT, NEXT_AT, 10, None));
    }

    #[test]
    fn dedups_same_occurrence() {
        let now = NEXT_AT - 300;
        assert!(!should_notify(now, NEXT_AT, 10, Some(NEXT_AT)));
    }

    #[test]
    fn re_arms_for_next_occurrence() {
        let prev = NEXT_AT;
        let next = NEXT_AT + 3600;
        assert!(should_notify(next - 300, next, 10, Some(prev)));
    }

    #[test]
    fn custom_lead_time() {
        assert!(should_notify(NEXT_AT - 900, NEXT_AT, 15, None));
        assert!(!should_notify(NEXT_AT - 901, NEXT_AT, 15, None));
    }

    #[test]
    fn capture_notifier_records() {
        let notifier = CapturingNotifier::new();
        notifier.send("t1", "b1").unwrap();
        notifier.send("t2", "b2").unwrap();
        assert_eq!(notifier.sent.lock().unwrap().len(), 2);
        assert_eq!(notifier.sent.lock().unwrap()[0], ("t1".into(), "b1".into()));
    }

    #[test]
    fn reminder_text_shape() {
        let now = NEXT_AT - 600;
        let (title, body) = reminder_text("Maghrib", NEXT_AT, now, "WLY01 Wilayah Persekutuan");
        assert!(title.starts_with("Waktu Solat — Maghrib in 10 minutes"), "{title}");
        assert!(body.starts_with("Maghrib at "), "{body}");
        assert!(body.ends_with(" · WLY01 Wilayah Persekutuan"), "{body}");
    }
}
