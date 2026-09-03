//! When to refresh, in what order, and how to back off.
//!
//! Everything here is a pure function of state and a clock reading. That is deliberate: a
//! scheduler that reads the wall clock itself cannot be tested at eight hours of simulated
//! offline time without waiting eight hours.
//!
//! **This module cannot write the default authentication.** Not by convention - it holds no
//! reference to a write path, and a test scans its source to keep it that way.

use std::time::Duration;

/// Backoff doubles from here.
const BACKOFF_BASE_SECONDS: u32 = 30;

/// And stops here. Beyond a quarter of an hour, retrying more often achieves
/// nothing except keeping a laptop awake.
pub const BACKOFF_CAP_SECONDS: u32 = 900;

/// When the panel opens, anything older than this is refreshed.
pub const EXPAND_REFRESH_AFTER_SECONDS: i64 = 120;

/// Consecutive failures, and the delay they earn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn failures(self) -> u32 {
        self.failures
    }

    /// Records a failure. Saturating, so a very long outage cannot overflow into a short delay.
    #[must_use]
    pub fn after_failure(self) -> Self {
        Self {
            failures: self.failures.saturating_add(1),
        }
    }

    /// A success clears the history: the next failure starts from the base delay again.
    #[must_use]
    pub fn after_success(self) -> Self {
        Self::new()
    }

    /// How long to wait before the next attempt.
    pub fn delay(self) -> Duration {
        if self.failures == 0 {
            return Duration::ZERO;
        }
        // `checked_shl` rather than `<<`: a long outage would otherwise shift past the width of
        // the type and wrap around to a tiny delay - the opposite of backing off.
        let doubled = BACKOFF_BASE_SECONDS
            .checked_shl(self.failures - 1)
            .unwrap_or(BACKOFF_CAP_SECONDS);
        Duration::from_secs(u64::from(doubled.min(BACKOFF_CAP_SECONDS)))
    }
}

/// What Toglet knows about one account's refresh state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshState {
    pub account_id: String,
    /// Unix seconds of the last successful reading, or `None` if there has never been one.
    pub last_success_at: Option<i64>,
    /// Unix seconds of the last attempt, successful or not.
    pub last_attempt_at: Option<i64>,
    pub backoff: Backoff,
    /// The account Codex is currently signed in as, which refreshes more often.
    pub is_active: bool,
    /// Accounts that cannot be read are not retried on a timer.
    pub is_refreshable: bool,
}

/// Why a refresh was scheduled. Useful in diagnostics and in reading a test's intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshTrigger {
    /// The regular interval elapsed.
    Interval,
    /// The panel was opened and this account's reading was old.
    PanelOpened,
    /// The user asked.
    Manual,
}

/// The intervals from settings.
#[derive(Debug, Clone, Copy)]
pub struct RefreshIntervals {
    pub active_seconds: u32,
    pub inactive_seconds: u32,
}

impl RefreshState {
    fn interval(&self, intervals: RefreshIntervals) -> i64 {
        i64::from(if self.is_active {
            intervals.active_seconds
        } else {
            intervals.inactive_seconds
        })
    }

    /// Whether the backoff delay from the last attempt has elapsed.
    fn backoff_elapsed(&self, now: i64) -> bool {
        match self.last_attempt_at {
            None => true,
            Some(attempted) => {
                let waited = (now - attempted).max(0);
                waited >= i64::try_from(self.backoff.delay().as_secs()).unwrap_or(i64::MAX)
            }
        }
    }

    fn age(&self, now: i64) -> Option<i64> {
        self.last_success_at.map(|at| (now - at).max(0))
    }

    /// Whether this account is due on the regular timer.
    pub fn is_due(&self, now: i64, intervals: RefreshIntervals) -> bool {
        if !self.is_refreshable || !self.backoff_elapsed(now) {
            return false;
        }
        match self.age(now) {
            // Never read: due immediately, once the backoff allows it.
            None => true,
            Some(age) => age >= self.interval(intervals),
        }
    }

    /// Whether opening the panel should refresh this account.
    pub fn is_due_on_expand(&self, now: i64) -> bool {
        if !self.is_refreshable || !self.backoff_elapsed(now) {
            return false;
        }
        self.age(now)
            .is_none_or(|age| age >= EXPAND_REFRESH_AFTER_SECONDS)
    }
}

/// Builds the ordered work list for a regular tick.
///
/// The result is a **queue**, not a set of parallel jobs. Concurrency is one by construction:
/// the caller walks this list one entry at a time, so at no point are two app servers running.
/// Ordering puts the active account first - it is the one on screen when the bar is collapsed.
pub fn due_now(
    states: &[RefreshState],
    now: i64,
    intervals: RefreshIntervals,
) -> Vec<(String, RefreshTrigger)> {
    ordered(
        states,
        |state| state.is_due(now, intervals),
        RefreshTrigger::Interval,
    )
}

/// Builds the ordered work list for the panel being opened.
pub fn due_on_expand(states: &[RefreshState], now: i64) -> Vec<(String, RefreshTrigger)> {
    ordered(
        states,
        |state| state.is_due_on_expand(now),
        RefreshTrigger::PanelOpened,
    )
}

/// Everything refreshable, for "refresh all". Still a queue, still sequential.
pub fn all_refreshable(states: &[RefreshState]) -> Vec<(String, RefreshTrigger)> {
    ordered(states, |state| state.is_refreshable, RefreshTrigger::Manual)
}

fn ordered(
    states: &[RefreshState],
    include: impl Fn(&RefreshState) -> bool,
    trigger: RefreshTrigger,
) -> Vec<(String, RefreshTrigger)> {
    let mut selected: Vec<&RefreshState> = states.iter().filter(|state| include(state)).collect();
    // Active first, then oldest first. A stable ordering also keeps the queue predictable in
    // tests, which matters when the thing being asserted is "one at a time, in this order".
    selected.sort_by_key(|state| (!state.is_active, state.last_success_at.unwrap_or(i64::MIN)));
    selected
        .into_iter()
        .map(|state| (state.account_id.clone(), trigger))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVALS: RefreshIntervals = RefreshIntervals {
        active_seconds: 60,
        inactive_seconds: 300,
    };

    fn state(id: &str, last_success_at: Option<i64>, is_active: bool) -> RefreshState {
        RefreshState {
            account_id: id.to_owned(),
            last_success_at,
            last_attempt_at: last_success_at,
            backoff: Backoff::new(),
            is_active,
            is_refreshable: true,
        }
    }

    #[test]
    fn the_active_account_is_due_after_its_shorter_interval() {
        let active = state("acct-1", Some(0), true);

        assert!(!active.is_due(59, INTERVALS));
        assert!(active.is_due(60, INTERVALS));
    }

    #[test]
    fn a_background_account_waits_for_the_longer_interval() {
        let inactive = state("acct-2", Some(0), false);

        assert!(!inactive.is_due(299, INTERVALS));
        assert!(inactive.is_due(300, INTERVALS));
    }

    #[test]
    fn an_account_never_read_is_due_immediately() {
        assert!(state("acct-1", None, false).is_due(0, INTERVALS));
    }

    #[test]
    fn an_account_that_cannot_be_read_is_never_scheduled() {
        let mut unsupported = state("acct-1", None, false);
        unsupported.is_refreshable = false;

        assert!(!unsupported.is_due(10_000, INTERVALS));
        assert!(!unsupported.is_due_on_expand(10_000));
        assert!(all_refreshable(&[unsupported]).is_empty());
    }

    #[test]
    fn opening_the_panel_refreshes_anything_older_than_two_minutes() {
        let recent = state("acct-1", Some(0), false);

        assert!(!recent.is_due_on_expand(119));
        assert!(recent.is_due_on_expand(120));
    }

    #[test]
    fn the_queue_puts_the_active_account_first_then_the_oldest() {
        let states = vec![
            state("old", Some(10), false),
            state("active", Some(500), true),
            state("older", Some(1), false),
        ];

        let queue = due_now(&states, 10_000, INTERVALS);

        let ids: Vec<&str> = queue.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["active", "older", "old"]);
    }

    #[test]
    fn the_backoff_doubles_and_then_stops_at_fifteen_minutes() {
        let mut backoff = Backoff::new();
        assert_eq!(backoff.delay(), Duration::ZERO);

        let expected = [30, 60, 120, 240, 480, 900, 900, 900];
        for want in expected {
            backoff = backoff.after_failure();
            assert_eq!(
                backoff.delay(),
                Duration::from_secs(want),
                "after {} failures",
                backoff.failures()
            );
        }
    }

    #[test]
    fn eight_hours_offline_does_not_produce_high_frequency_retries() {
        // On a simulated clock. Walking the real thing would take eight hours.
        let mut current = state("acct-1", Some(0), true);
        current.last_success_at = None;
        let mut now = 0i64;
        let mut attempts = 0;

        while now < 8 * 60 * 60 {
            if current.is_due(now, INTERVALS) {
                attempts += 1;
                current.last_attempt_at = Some(now);
                current.backoff = current.backoff.after_failure();
            }
            now += 1;
        }

        // Unbounded retrying at the 60 s interval would be 480 attempts. With the cap at
        // 15 minutes the tail settles to four an hour.
        assert!(
            attempts <= 40,
            "eight hours offline produced {attempts} attempts"
        );
        assert_eq!(current.backoff.delay(), Duration::from_secs(900));
    }

    #[test]
    fn a_success_clears_the_backoff_so_the_next_outage_starts_short() {
        let backoff = Backoff::new()
            .after_failure()
            .after_failure()
            .after_failure();
        assert_eq!(backoff.delay(), Duration::from_secs(120));

        let recovered = backoff.after_success();

        assert_eq!(recovered.failures(), 0);
        assert_eq!(recovered.after_failure().delay(), Duration::from_secs(30));
    }

    #[test]
    fn a_very_long_outage_does_not_wrap_around_to_a_short_delay() {
        let mut backoff = Backoff::new();
        for _ in 0..200 {
            backoff = backoff.after_failure();
        }

        assert_eq!(
            backoff.delay(),
            Duration::from_secs(u64::from(BACKOFF_CAP_SECONDS)),
            "shifting past the width of the type must not produce a tiny delay"
        );
    }

    #[test]
    fn an_account_still_inside_its_backoff_is_not_scheduled() {
        let mut failing = state("acct-1", Some(0), true);
        failing.backoff = failing.backoff.after_failure().after_failure(); // 60 s
        failing.last_attempt_at = Some(1_000);

        assert!(!failing.is_due(1_059, INTERVALS));
        assert!(failing.is_due(1_060, INTERVALS));
    }

    #[test]
    fn a_clock_jumping_backwards_does_not_make_everything_due() {
        let recent = state("acct-1", Some(1_000), true);

        // Waking up with a corrected clock: the age is clamped at zero, not negative.
        assert!(!recent.is_due(500, INTERVALS));
    }

    /// Refreshing must not be able to touch the default authentication.
    ///
    /// A source scan, for the same reason as the `SwitchVerified` guard: the language cannot
    /// express "this module may not call that one". What it can do is fail the build when
    /// somebody adds the call.
    #[test]
    fn the_quota_module_holds_no_path_to_writing_the_default_authentication() {
        let quota = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/quota");
        let mut offenders = Vec::new();

        for entry in std::fs::read_dir(&quota).expect("the quota module is readable") {
            let path = entry.expect("entry is readable").path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("source is readable");
            let production = text.split("#[cfg(test)]").next().unwrap_or_default();
            for forbidden in ["atomic_write", "write_private_file", "SwitchVerified"] {
                if production.contains(forbidden) {
                    offenders.push(format!("{} uses {forbidden}", path.display()));
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "the refresh path must not be able to write authentication: {offenders:?}"
        );
    }
}
