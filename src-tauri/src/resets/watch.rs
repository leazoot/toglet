//! Which reading deserves a notification: a reset, an announcement or a forecast that was not
//! there the last time. Pure, so every moment can be tested without a feed.

use serde::{Deserialize, Serialize};

use super::feed::{ResetKind, ResetStatus, WatchLevel};

/// What was last seen, so the same event is announced once. Persisted, so a restart does not
/// announce it again.
///
/// `seen` is separate from the three keys on purpose: a feed with nothing in it (`false`, all
/// `None`) and a feed that has never been read (`false`, all `None`) would otherwise be the
/// same value, and the first reading after switching on would announce a reset from last week.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Markers {
    /// Whether any reading has been recorded since the feature was switched on.
    pub seen: bool,
    pub reset_id: Option<String>,
    pub scheduled_id: Option<String>,
    /// A forecast has no id; its observation time is what tells two apart.
    pub watch_observed_at: Option<i64>,
}

/// One event worth telling the person about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Announcement {
    /// A reset was executed (or observed to have been).
    Reset { kind: ResetKind, at: i64 },
    /// A reset was announced for later.
    Scheduled {
        kind: ResetKind,
        at: i64,
        scheduled_for: Option<i64>,
    },
    /// The feed's classifier sees a reset coming.
    Watch {
        level: WatchLevel,
        chance_percent: Option<u8>,
        at: i64,
    },
}

/// Compares a reading with what was last seen. Returns the markers to store and the events to
/// announce, in the order the feed lists them.
///
/// The first reading after switching on records everything and announces nothing: a reset
/// from five days ago is the current state, not news.
pub fn announce(before: &Markers, status: &ResetStatus) -> (Markers, Vec<Announcement>) {
    let after = Markers {
        seen: true,
        reset_id: status.latest_reset.as_ref().map(|reset| reset.id.clone()),
        scheduled_id: status
            .scheduled_reset
            .as_ref()
            .map(|scheduled| scheduled.id.clone()),
        watch_observed_at: status.active_watch.as_ref().map(|watch| watch.observed_at),
    };
    if !before.seen {
        return (after, Vec::new());
    }

    let mut events = Vec::new();
    if let Some(reset) = &status.latest_reset
        && before.reset_id.as_deref() != Some(&reset.id)
    {
        events.push(Announcement::Reset {
            kind: reset.kind,
            at: reset.announced_at,
        });
    }
    if let Some(scheduled) = &status.scheduled_reset
        && before.scheduled_id.as_deref() != Some(&scheduled.id)
    {
        events.push(Announcement::Scheduled {
            kind: scheduled.kind,
            at: scheduled.announced_at,
            scheduled_for: scheduled.scheduled_for,
        });
    }
    if let Some(watch) = &status.active_watch
        && before.watch_observed_at != Some(watch.observed_at)
    {
        events.push(Announcement::Watch {
            level: watch.level,
            chance_percent: watch.chance_percent,
            at: watch.observed_at,
        });
    }
    (after, events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resets::feed::{Reset, Scheduled, Stats, Watch};

    fn reading(id: &str) -> ResetStatus {
        ResetStatus {
            latest_reset: Some(Reset {
                id: id.to_owned(),
                kind: ResetKind::Regular,
                announced_at: 1_789_200_557,
                text: None,
            }),
            scheduled_reset: None,
            active_watch: None,
            stats: Stats {
                total: 53,
                last_reset_at: Some(1_789_200_557),
                days_since_last: Some(4.9),
                avg_interval_days: Some(6.9),
            },
            generated_at: 1_789_626_354,
        }
    }

    #[test]
    fn the_first_reading_records_everything_and_announces_nothing() {
        let (after, events) = announce(&Markers::default(), &reading("r1"));
        assert!(events.is_empty());
        assert!(after.seen);
        assert_eq!(after.reset_id.as_deref(), Some("r1"));
    }

    #[test]
    fn a_reading_with_nothing_in_it_still_counts_as_seen() {
        let mut empty = reading("r1");
        empty.latest_reset = None;
        let (after, events) = announce(&Markers::default(), &empty);
        assert!(events.is_empty());
        assert!(
            after.seen,
            "an empty feed and an unread feed must not look the same"
        );
    }

    #[test]
    fn the_same_reset_again_is_not_news() {
        let (markers, _) = announce(&Markers::default(), &reading("r1"));
        let (_, events) = announce(&markers, &reading("r1"));
        assert!(events.is_empty());
    }

    #[test]
    fn a_new_reset_id_is_announced_once() {
        let (markers, _) = announce(&Markers::default(), &reading("r1"));
        let (markers, events) = announce(&markers, &reading("r2"));
        assert_eq!(
            events,
            vec![Announcement::Reset {
                kind: ResetKind::Regular,
                at: 1_789_200_557
            }]
        );
        let (_, again) = announce(&markers, &reading("r2"));
        assert!(again.is_empty());
    }

    #[test]
    fn an_announcement_and_a_forecast_are_each_announced_by_their_own_key() {
        let (markers, _) = announce(&Markers::default(), &reading("r1"));
        let mut next = reading("r1");
        next.scheduled_reset = Some(Scheduled {
            id: "s1".to_owned(),
            kind: ResetKind::Banked,
            announced_at: 10,
            scheduled_for: None,
            text: None,
        });
        next.active_watch = Some(Watch {
            level: WatchLevel::Elevated,
            chance_percent: None,
            forecast_window: "next 48h".to_owned(),
            observed_at: 20,
            expires_at: 30,
            text: None,
        });
        let (markers, events) = announce(&markers, &next);
        assert_eq!(
            events,
            vec![
                Announcement::Scheduled {
                    kind: ResetKind::Banked,
                    at: 10,
                    scheduled_for: None
                },
                Announcement::Watch {
                    level: WatchLevel::Elevated,
                    chance_percent: None,
                    at: 20
                },
            ]
        );
        // The same forecast re-observed later is a new one; the same announcement is not.
        let mut later = next.clone();
        later.active_watch.as_mut().expect("watch").observed_at = 21;
        let (_, events) = announce(&markers, &later);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Announcement::Watch { at: 21, .. }));
    }

    #[test]
    fn a_forecast_that_expired_leaves_no_event_and_clears_its_marker() {
        let mut with_watch = reading("r1");
        with_watch.active_watch = Some(Watch {
            level: WatchLevel::Strong,
            chance_percent: Some(70),
            forecast_window: "today".to_owned(),
            observed_at: 20,
            expires_at: 30,
            text: None,
        });
        let (markers, _) = announce(&Markers::default(), &with_watch);
        let (markers, events) = announce(&markers, &reading("r1"));
        assert!(events.is_empty());
        assert_eq!(markers.watch_observed_at, None);
    }
}
