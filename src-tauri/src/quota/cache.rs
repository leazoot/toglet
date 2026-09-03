//! Quota snapshots and what makes one stale.
//!
//! **`stale` is derived, never stored.** It appears as a field in the wire form, but keeping
//! it as state would create exactly the forbidden path: something setting `stale = false`
//! without a new reading behind it. Here staleness is a function of `fetched_at` and the clock,
//! so making a snapshot look fresh requires actually fetching.
//!
//! A stale snapshot keeps its values. Cached and unknown are different things, and an expired
//! reading is still the last thing that was true.

use serde::Serialize;

use super::normalize::NormalisedQuota;

/// How long a reading stays fresh.
pub const STALE_AFTER_SECONDS: i64 = 600;

/// Where a reading came from. One value today; a constant rather than a free string so a
/// future source has to be added deliberately.
pub const SOURCE_APP_SERVER: &str = "codex_app_server";

/// The last thing known about one account's quota.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaSnapshot {
    account_id: String,
    quota: NormalisedQuota,
    /// Unix seconds of the last **successful** read. The only thing that moves it is a new
    /// reading.
    fetched_at: i64,
    /// The error from the most recent failed attempt, if the last attempt failed.
    last_error_code: Option<String>,
}

impl QuotaSnapshot {
    /// Records a successful reading.
    pub fn fresh(account_id: &str, quota: NormalisedQuota, now: i64) -> Self {
        Self {
            account_id: account_id.to_owned(),
            quota,
            fetched_at: now,
            last_error_code: None,
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn quota(&self) -> &NormalisedQuota {
        &self.quota
    }

    pub fn fetched_at(&self) -> i64 {
        self.fetched_at
    }

    pub fn last_error_code(&self) -> Option<&str> {
        self.last_error_code.as_deref()
    }

    /// Records a failed attempt.
    ///
    /// The values and `fetched_at` are untouched: a failure does not make the last successful
    /// reading untrue, it makes it older. Clearing them here is what would turn a network blip
    /// into an empty panel.
    pub fn record_failure(&mut self, error_code: &str) {
        self.last_error_code = Some(error_code.to_owned());
    }

    /// Records a new successful reading, which is the only way to become fresh again.
    pub fn record_success(&mut self, quota: NormalisedQuota, now: i64) {
        self.quota = quota;
        self.fetched_at = now;
        self.last_error_code = None;
    }

    /// How old the reading is, never negative.
    ///
    /// A clock that jumps backwards - a laptop waking up, an NTP correction - would otherwise
    /// produce a negative age and, through it, a snapshot that looks impossibly fresh.
    pub fn age_seconds(&self, now: i64) -> i64 {
        (now - self.fetched_at).max(0)
    }

    /// Whether the reading has passed its freshness window.
    pub fn is_stale(&self, now: i64) -> bool {
        self.age_seconds(now) > STALE_AFTER_SECONDS
    }

    /// The form the frontend receives, with staleness resolved against `now`.
    pub fn view(&self, now: i64) -> QuotaSnapshotView<'_> {
        QuotaSnapshotView {
            account_id: &self.account_id,
            quota: &self.quota,
            fetched_at: self.fetched_at,
            source: SOURCE_APP_SERVER,
            stale: self.is_stale(now),
            last_error_code: self.last_error_code.as_deref(),
        }
    }
}

/// The five things a snapshot must carry, plus the values themselves.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshotView<'a> {
    pub account_id: &'a str,
    pub quota: &'a NormalisedQuota,
    pub fetched_at: i64,
    pub source: &'static str,
    pub stale: bool,
    pub last_error_code: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_server::{RawRateLimits, RawWindow};

    fn quota(used: f64) -> NormalisedQuota {
        NormalisedQuota::from_raw(&RawRateLimits {
            primary: Some(RawWindow {
                used_percent: used,
                window_duration_mins: Some(300),
                resets_at: Some(2_000),
            }),
            secondary: None,
            plan_type: Some("plus".to_owned()),
        })
    }

    #[test]
    fn a_new_reading_is_fresh_and_carries_all_five_fields() {
        let snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 1_000);

        let view = snapshot.view(1_000);
        assert_eq!(view.account_id, "acct-1");
        assert_eq!(view.fetched_at, 1_000);
        assert_eq!(view.source, SOURCE_APP_SERVER);
        assert!(!view.stale);
        assert_eq!(view.last_error_code, None);
    }

    #[test]
    fn a_reading_goes_stale_exactly_after_the_freshness_window() {
        let snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 0);

        assert!(!snapshot.is_stale(STALE_AFTER_SECONDS));
        assert!(snapshot.is_stale(STALE_AFTER_SECONDS + 1));
    }

    #[test]
    fn a_stale_reading_keeps_its_values() {
        let snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 0);

        let view = snapshot.view(STALE_AFTER_SECONDS + 1);

        assert!(view.stale);
        assert_eq!(
            view.quota
                .five_hour()
                .expect("still present")
                .remaining_percent,
            98.0,
            "an expired reading is still the last thing that was true"
        );
    }

    #[test]
    fn a_failure_ages_the_reading_without_erasing_it() {
        let mut snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 0);

        snapshot.record_failure("network_unavailable");

        let view = snapshot.view(10);
        assert_eq!(view.last_error_code, Some("network_unavailable"));
        assert_eq!(
            view.fetched_at, 0,
            "a failure must not pretend to be a reading"
        );
        assert_eq!(
            view.quota
                .five_hour()
                .expect("still present")
                .remaining_percent,
            98.0
        );
    }

    #[test]
    fn only_a_new_reading_can_make_a_snapshot_fresh_again() {
        let mut snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 0);
        assert!(snapshot.is_stale(STALE_AFTER_SECONDS + 1));

        snapshot.record_success(quota(3.0), STALE_AFTER_SECONDS + 1);

        assert!(!snapshot.is_stale(STALE_AFTER_SECONDS + 1));
        assert_eq!(snapshot.fetched_at(), STALE_AFTER_SECONDS + 1);
        assert_eq!(
            snapshot.last_error_code(),
            None,
            "success clears the last error"
        );
    }

    #[test]
    fn there_is_no_way_to_clear_staleness_without_a_reading() {
        // `stale` is not a field; it is `age_seconds(now) > STALE_AFTER_SECONDS`. The only
        // thing that moves `fetched_at` is `fresh` or `record_success`, both of which take a
        // quota. This test documents that as an intentional property rather than an accident.
        let mut snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 0);
        snapshot.record_failure("network_unavailable");

        assert!(
            snapshot.is_stale(STALE_AFTER_SECONDS + 1),
            "recording failures must not refresh the snapshot"
        );
    }

    #[test]
    fn a_clock_jumping_backwards_does_not_produce_a_negative_age() {
        let snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 1_000);

        // A laptop waking up with a corrected clock.
        assert_eq!(snapshot.age_seconds(500), 0);
        assert!(!snapshot.is_stale(500));
    }

    #[test]
    fn the_serialised_view_names_its_source() {
        let snapshot = QuotaSnapshot::fresh("acct-1", quota(2.0), 0);

        let json = serde_json::to_string(&snapshot.view(0)).expect("serialises");

        assert!(json.contains("\"source\":\"codex_app_server\""));
        assert!(json.contains("\"stale\":false"));
    }
}
