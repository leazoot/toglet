//! Turning what the server reported into the windows Toglet displays.
//!
//! This is the module to be most careful in: it is where "no value" quietly becomes `0%`
//! if nobody is careful. The type system is set up so that it cannot:
//!
//! * A window that did not arrive is **absent**, not a window with zero usage.
//! * A window whose duration the server did not state is [`WindowKind::Unknown`], **not**
//!   five-hour. The slot it arrived in (`primary`/`secondary`) says nothing about its type.
//! * Percentages are `f64` all the way through; rounding for display is the frontend's job, so
//!   nothing is thrown away here.

use serde::Serialize;

use crate::app_server::{RawRateLimits, RawWindow};

/// The window lengths Toglet gives meaning to.
const FIVE_HOUR_MINUTES: i64 = 300;
const WEEKLY_MINUTES: i64 = 10_080;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    Weekly,
    /// A window with a duration Toglet has no meaning for. Kept, not discarded - the server
    /// said something and hiding it would be its own kind of dishonesty.
    Other,
    /// The server did not state a duration. The type is genuinely unknown; guessing would be
    /// how a weekly window ends up displayed as a five-hour one.
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

/// One normalised window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub kind: WindowKind,
    /// `None` when the server did not say.
    pub duration_minutes: Option<i64>,
    /// As reported, unclamped. Kept alongside `remaining_percent` so a nonsensical reading
    /// stays visible rather than being silently corrected out of existence.
    pub used_percent: f64,
    /// `clamp(100 - used, 0, 100)`.
    pub remaining_percent: f64,
    /// Unix seconds, absolute. `None` when the server did not say; the display layer converts
    /// to local time.
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

    /// Seconds until this window resets, never negative.
    ///
    /// A reset time in the past means the window has already rolled over and the server has not
    /// told us yet; `0` says "any moment now". A negative countdown would be nonsense on screen,
    /// and a sleeping laptop makes it a routine occurrence rather than an edge case.
    pub fn seconds_until_reset(&self, now: i64) -> Option<i64> {
        self.resets_at.map(|at| (at - now).max(0))
    }
}

/// `clamp(100 - used, 0, 100)`.
///
/// The clamp is for the server's benefit, not ours: a reading above 100 or below 0 is not
/// something to propagate into a progress ring.
pub fn remaining_percent(used_percent: f64) -> f64 {
    if used_percent.is_nan() {
        // Not a number is not zero usage. Nothing can be said about the remainder, and the
        // caller sees that through `windows` rather than through a fabricated 0 or 100.
        return f64::NAN;
    }
    (100.0 - used_percent).clamp(0.0, 100.0)
}

/// The windows one account currently has.
///
/// A window that is missing is **missing**. There is no constructor that fills in a placeholder,
/// which keeps that true by construction rather than by review.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalisedQuota {
    pub windows: Vec<QuotaWindow>,
    /// `None` when the server did not say, or said `"unknown"`.
    pub plan_type: Option<String>,
}

impl NormalisedQuota {
    /// Normalises everything the server returned.
    ///
    /// Both slots are walked and classified independently; neither position implies a type.
    pub fn from_raw(raw: &RawRateLimits) -> Self {
        let windows = [raw.primary.as_ref(), raw.secondary.as_ref()]
            .into_iter()
            .flatten()
            .map(QuotaWindow::from_raw)
            .collect();

        Self {
            windows,
            plan_type: raw.plan_type.clone(),
        }
    }

    /// The window of a given kind, if the server returned one.
    ///
    /// `None` means **not returned**. The caller shows "not returned"; it does not show `0%`.
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
        }
    }

    #[test]
    fn the_measured_payload_classifies_both_windows() {
        // The exact measured values.
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
        // Rounding is the display layer's decision; throwing precision away here would make it
        // impossible to show one decimal place later.
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
