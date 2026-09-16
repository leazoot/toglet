//! Turning what the server reported into the windows Toglet displays.
//!
//! A window that did not arrive is absent, never zero usage; a window without a stated duration
//! is `Unknown`, whatever slot it arrived in. Rounding is left to the frontend.

use serde::Serialize;

use crate::app_server::{RawRateLimits, RawResetCredits, RawWindow};

const FIVE_HOUR_MINUTES: i64 = 300;
const WEEKLY_MINUTES: i64 = 10_080;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    Weekly,
    /// A duration Toglet has no meaning for; kept rather than discarded.
    Other,
    /// The server did not state a duration.
    Unknown,
}

impl WindowKind {
    /// Classifies by duration and by nothing else.
    pub fn from_duration(minutes: Option<i64>) -> Self {
        match minutes {
            Some(FIVE_HOUR_MINUTES) => Self::FiveHour,
            Some(WEEKLY_MINUTES) => Self::Weekly,
            Some(_) => Self::Other,
            None => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FiveHour => "five_hour",
            Self::Weekly => "weekly",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub kind: WindowKind,
    /// `None` when the server did not say.
    pub duration_minutes: Option<i64>,
    /// As reported, unclamped, so a nonsensical reading stays visible.
    pub used_percent: f64,
    pub remaining_percent: f64,
    /// Absolute Unix seconds; `None` when the server did not say.
    pub resets_at: Option<i64>,
}

impl QuotaWindow {
    fn from_raw(raw: &RawWindow) -> Self {
        Self {
            kind: WindowKind::from_duration(raw.window_duration_mins),
            duration_minutes: raw.window_duration_mins,
            used_percent: raw.used_percent,
            remaining_percent: remaining_percent(raw.used_percent),
            resets_at: raw.resets_at,
        }
    }

    /// Seconds until this window resets, clamped at zero once the reset time has passed.
    pub fn seconds_until_reset(&self, now: i64) -> Option<i64> {
        self.resets_at.map(|at| (at - now).max(0))
    }
}

/// `clamp(100 - used, 0, 100)`; a NaN reading stays NaN.
pub fn remaining_percent(used_percent: f64) -> f64 {
    if used_percent.is_nan() {
        // Not a number is not zero usage; do not fabricate a 0 or 100.
        return f64::NAN;
    }
    (100.0 - used_percent).clamp(0.0, 100.0)
}

/// The windows one account currently has; a missing window is simply absent.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalisedQuota {
    pub windows: Vec<QuotaWindow>,
    /// `None` when the server did not say, or said `"unknown"`.
    pub plan_type: Option<String>,
    /// `None` when the server never mentioned reset credits, which is what servers before 0.154
    /// do. That is "not reported", and must not be shown as holding none.
    pub reset_credits: Option<ResetCredits>,
}

/// The reset credits an account holds. Redeeming one clears the rate-limit windows.
///
/// Not the `credits` balance reported beside it: that is purchased usage, which Toglet reads but
/// deliberately does not interpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredits {
    /// The server's own count of redeemable credits.
    pub available_count: i64,
    /// Unix seconds of the next expiry among the details that arrived; `None` when none came or
    /// none of them expire. The server may cap that list, so it is not always the true earliest.
    pub earliest_expiry: Option<i64>,
}

impl ResetCredits {
    fn from_raw(raw: &RawResetCredits) -> Self {
        Self {
            available_count: raw.available_count,
            earliest_expiry: raw.earliest_expiry,
        }
    }

    /// Whether there is anything worth showing. A zero count is not shown at all.
    pub fn any(&self) -> bool {
        self.available_count > 0
    }
}

impl NormalisedQuota {
    /// Classifies both slots independently; neither position implies a type.
    pub fn from_raw(raw: &RawRateLimits) -> Self {
        let windows = [raw.primary.as_ref(), raw.secondary.as_ref()]
            .into_iter()
            .flatten()
            .map(QuotaWindow::from_raw)
            .collect();

        Self {
            windows,
            plan_type: raw.plan_type.clone(),
            reset_credits: raw.reset_credits.as_ref().map(ResetCredits::from_raw),
        }
    }

    /// The window of a given kind; `None` means not returned, never `0%`.
    pub fn window(&self, kind: WindowKind) -> Option<&QuotaWindow> {
        self.windows.iter().find(|window| window.kind == kind)
    }

    pub fn five_hour(&self) -> Option<&QuotaWindow> {
        self.window(WindowKind::FiveHour)
    }

    pub fn weekly(&self) -> Option<&QuotaWindow> {
        self.window(WindowKind::Weekly)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(used: f64, duration: Option<i64>, resets_at: Option<i64>) -> RawWindow {
        RawWindow {
            used_percent: used,
            window_duration_mins: duration,
            resets_at,
        }
    }

    fn limits(primary: Option<RawWindow>, secondary: Option<RawWindow>) -> RawRateLimits {
        RawRateLimits {
            primary,
            secondary,
            plan_type: Some("plus".to_owned()),
            credits: None,
            reset_credits: None,
            by_limit_id: Default::default(),
        }
    }

    #[test]
    fn the_measured_payload_classifies_both_windows() {
        let normalised = NormalisedQuota::from_raw(&limits(
            Some(window(2.0, Some(300), Some(1_788_164_992))),
            Some(window(0.0, Some(10_080), Some(1_788_751_792))),
        ));

        assert_eq!(
            normalised.five_hour().expect("present").remaining_percent,
            98.0
        );
        assert_eq!(
            normalised.weekly().expect("present").remaining_percent,
            100.0
        );
    }

    #[test]
    fn a_weekly_window_in_the_primary_slot_is_still_weekly() {
        // The slot says nothing. Only `windowDurationMins` does.
        let normalised = NormalisedQuota::from_raw(&limits(
            Some(window(10.0, Some(10_080), None)),
            Some(window(20.0, Some(300), None)),
        ));

        assert_eq!(normalised.weekly().expect("present").used_percent, 10.0);
        assert_eq!(normalised.five_hour().expect("present").used_percent, 20.0);
    }

    #[test]
    fn a_missing_weekly_window_is_absent_and_never_zero() {
        let normalised =
            NormalisedQuota::from_raw(&limits(Some(window(5.0, Some(300), None)), None));

        assert!(
            normalised.weekly().is_none(),
            "a window the server did not return must not become 0%"
        );
        assert_eq!(normalised.windows.len(), 1);
    }

    #[test]
    fn a_window_without_a_duration_is_unknown_not_five_hour() {
        let normalised = NormalisedQuota::from_raw(&limits(Some(window(5.0, None, None)), None));

        assert_eq!(normalised.windows[0].kind, WindowKind::Unknown);
        assert!(normalised.five_hour().is_none());
        assert!(normalised.weekly().is_none());
    }

    #[test]
    fn an_unrecognised_duration_is_kept_as_other_rather_than_discarded() {
        let normalised =
            NormalisedQuota::from_raw(&limits(Some(window(5.0, Some(1440), None)), None));

        assert_eq!(normalised.windows[0].kind, WindowKind::Other);
        assert_eq!(normalised.windows[0].duration_minutes, Some(1440));
    }

    #[test]
    fn the_remaining_percentage_covers_both_ends_and_beyond() {
        assert_eq!(remaining_percent(0.0), 100.0);
        assert_eq!(remaining_percent(100.0), 0.0);
        // A server that reports nonsense does not get to drive the ring past its ends.
        assert_eq!(remaining_percent(-5.0), 100.0);
        assert_eq!(remaining_percent(120.0), 0.0);
    }

    #[test]
    fn a_fraction_survives_normalisation() {
        // Rounding is the display layer's decision.
        let normalised =
            NormalisedQuota::from_raw(&limits(Some(window(2.5, Some(300), None)), None));

        assert_eq!(
            normalised.five_hour().expect("present").remaining_percent,
            97.5
        );
    }

    #[test]
    fn the_unclamped_reading_stays_visible_next_to_the_clamped_one() {
        let normalised =
            NormalisedQuota::from_raw(&limits(Some(window(120.0, Some(300), None)), None));
        let five_hour = normalised.five_hour().expect("present");

        assert_eq!(five_hour.used_percent, 120.0, "the raw reading is kept");
        assert_eq!(five_hour.remaining_percent, 0.0);
    }

    #[test]
    fn a_reading_that_is_not_a_number_does_not_become_zero_or_full() {
        let remaining = remaining_percent(f64::NAN);

        assert!(
            remaining.is_nan(),
            "an unreadable value must not be turned into 0 or 100"
        );
    }

    #[test]
    fn a_reset_time_already_past_counts_down_to_zero_not_below() {
        let normalised =
            NormalisedQuota::from_raw(&limits(Some(window(5.0, Some(300), Some(1_000))), None));

        let window = normalised.five_hour().expect("present");
        assert_eq!(window.seconds_until_reset(500), Some(500));
        assert_eq!(
            window.seconds_until_reset(9_999),
            Some(0),
            "a laptop waking up must not produce a negative countdown"
        );
    }

    #[test]
    fn a_window_without_a_reset_time_reports_no_countdown_rather_than_zero() {
        let normalised =
            NormalisedQuota::from_raw(&limits(Some(window(5.0, Some(300), None)), None));

        assert_eq!(
            normalised
                .five_hour()
                .expect("present")
                .seconds_until_reset(0),
            None
        );
    }

    #[test]
    fn no_windows_at_all_yields_no_windows_rather_than_two_empty_ones() {
        let normalised = NormalisedQuota::from_raw(&limits(None, None));

        assert!(normalised.windows.is_empty());
        assert!(normalised.five_hour().is_none());
        assert!(normalised.weekly().is_none());
    }

    #[test]
    fn the_serialised_form_carries_no_placeholder_for_a_missing_window() {
        let normalised =
            NormalisedQuota::from_raw(&limits(Some(window(2.0, Some(300), None)), None));

        let json = serde_json::to_string(&normalised).expect("serialises");

        assert!(json.contains("\"kind\":\"five_hour\""));
        assert!(
            !json.contains("weekly"),
            "a missing window must not appear at all"
        );
    }
}
