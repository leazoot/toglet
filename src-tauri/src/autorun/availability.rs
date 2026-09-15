//! Whether an account can carry the bound task now, when it could, and which account to use.
//!
//! Pure functions over gathered facts. The credit balance is deliberately ignored: accounts
//! without purchased credits report `hasCredits: false` yet run turns normally.
//! The expected recovery time is the maximum over all blockers; one blocker without a time
//! means no time.

use crate::accounts::AccountStatus;
use crate::app_server::TurnErrorKind;
use crate::quota::{QuotaSnapshot, STALE_AFTER_SECONDS, WindowKind};

/// Everything known about one participating account at the moment of judgement.
#[derive(Debug, Clone)]
pub struct AccountFacts<'a> {
    pub status: AccountStatus,
    /// The last successful reading, or `None` when there has never been one.
    pub quota: Option<&'a QuotaSnapshot>,
    /// How the bound thread's most recent turn ended **on this account**, when it ran here.
    /// `None` for an account that has not run the task or whose last turn did not fail.
    pub last_turn_error: Option<TurnErrorKind>,
}

/// One reason an account cannot carry the task right now, with its recovery time when the
/// server gave one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    ReauthRequired,
    Unsupported,
    FiveHourExhausted {
        resets_at: Option<i64>,
    },
    WeeklyExhausted {
        resets_at: Option<i64>,
    },
    /// The last turn reported `usageLimitExceeded` while the quota shows room: the turn error is
    /// trusted over the numbers, and the five-hour reset is the only time to wait for.
    ServerLimited {
        resets_at: Option<i64>,
    },
    /// No reading, a reading missing a window, or a reading that says nothing. Not permanent:
    /// the account is re-queried after a backoff rather than written off.
    QuotaUnknown,
}

impl Blocker {
    /// When this blocker lifts, if the server said. `Some(None)` is "never on its own";
    /// `None` is "unknown", which for the expected time is the same thing.
    fn recovers_at(self) -> Option<i64> {
        match self {
            Self::FiveHourExhausted { resets_at }
            | Self::WeeklyExhausted { resets_at }
            | Self::ServerLimited { resets_at } => resets_at,
            Self::ReauthRequired | Self::Unsupported | Self::QuotaUnknown => None,
        }
    }

    /// Whether a fresh quota read could remove this blocker without anyone doing anything.
    pub fn is_recheckable(self) -> bool {
        matches!(self, Self::QuotaUnknown)
    }

    /// Whether waiting is enough. An exhausted window reopens on its own even without a reset
    /// time; an expired sign-in or an unreadable plan does not.
    pub fn lifts_on_its_own(self) -> bool {
        match self {
            Self::FiveHourExhausted { .. }
            | Self::WeeklyExhausted { .. }
            | Self::ServerLimited { .. }
            | Self::QuotaUnknown => true,
            Self::ReauthRequired | Self::Unsupported => false,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReauthRequired => "reauth_required",
            Self::Unsupported => "unsupported",
            Self::FiveHourExhausted { .. } => "five_hour_exhausted",
            Self::WeeklyExhausted { .. } => "weekly_exhausted",
            Self::ServerLimited { .. } => "server_limited",
            Self::QuotaUnknown => "quota_unknown",
        }
    }

    /// The inverse of [`as_str`](Self::as_str). A blocker read back carries no reset time: the
    /// code does not, and the expected time is stored on its own.
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "reauth_required" => Self::ReauthRequired,
            "unsupported" => Self::Unsupported,
            "five_hour_exhausted" => Self::FiveHourExhausted { resets_at: None },
            "weekly_exhausted" => Self::WeeklyExhausted { resets_at: None },
            "server_limited" => Self::ServerLimited { resets_at: None },
            "quota_unknown" => Self::QuotaUnknown,
            _ => return None,
        })
    }
}

/// The blockers of one account and when they will all have lifted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    /// Never empty.
    pub blockers: Vec<Blocker>,
    /// The **latest** recovery time among the blockers. `None` as soon as one blocker has no
    /// time: such an account has no expected time and takes no part in "earliest available".
    pub expected_available_at: Option<i64>,
}

impl Blocked {
    /// Whether every blocker could be cleared by reading the quota again.
    pub fn is_recheckable(&self) -> bool {
        self.blockers.iter().all(|blocker| blocker.is_recheckable())
    }

    /// Whether every blocker lifts with time alone.
    pub fn lifts_on_its_own(&self) -> bool {
        self.blockers
            .iter()
            .all(|blocker| blocker.lifts_on_its_own())
    }
}

/// The judgement for one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// No blockers and a reading younger than the freshness window.
    Available,
    /// No blockers, but the reading is too old to act on: re-query first.
    Recheck,
    Blocked(Blocked),
}

/// Judges one account. `now` is unix seconds, supplied by the caller so the rule is testable.
pub fn assess(facts: &AccountFacts<'_>, now: i64) -> Verdict {
    let mut blockers = Vec::new();

    match facts.status {
        AccountStatus::ReauthRequired => blockers.push(Blocker::ReauthRequired),
        AccountStatus::Unsupported => blockers.push(Blocker::Unsupported),
        // Every other status is about the reading or the switch, not the account itself; the
        // reading is judged below.
        AccountStatus::Ready
        | AccountStatus::Active
        | AccountStatus::Refreshing
        | AccountStatus::Stale
        | AccountStatus::Offline
        | AccountStatus::Switching
        | AccountStatus::Error => {}
    }

    let Some(snapshot) = facts.quota else {
        blockers.push(Blocker::QuotaUnknown);
        return Verdict::Blocked(blocked(blockers));
    };

    let five_hour = window_blocker(snapshot, WindowKind::FiveHour);
    let weekly = window_blocker(snapshot, WindowKind::Weekly);
    let five_hour_resets_at = snapshot.quota().five_hour().and_then(|w| w.resets_at);

    match (five_hour, weekly) {
        // A window the server did not return, or returned without a usable number, is not
        // "0% used" and not "100% used": it is unknown.
        (WindowState::Unknown, _) | (_, WindowState::Unknown) => {
            blockers.push(Blocker::QuotaUnknown);
        }
        (five_hour, weekly) => {
            let mut exhausted = false;
            if five_hour == WindowState::Exhausted {
                blockers.push(Blocker::FiveHourExhausted {
                    resets_at: five_hour_resets_at,
                });
                exhausted = true;
            }
            if weekly == WindowState::Exhausted {
                blockers.push(Blocker::WeeklyExhausted {
                    resets_at: snapshot.quota().weekly().and_then(|w| w.resets_at),
                });
                exhausted = true;
            }
            // The turn's own verdict outranks the percentages: a usage-limit failure with both
            // windows showing room is a limit the numbers cannot see.
            if !exhausted && facts.last_turn_error == Some(TurnErrorKind::UsageLimitExceeded) {
                blockers.push(Blocker::ServerLimited {
                    resets_at: five_hour_resets_at,
                });
            }
        }
    }

    if !blockers.is_empty() {
        return Verdict::Blocked(blocked(blockers));
    }
    if snapshot.age_seconds(now) > STALE_AFTER_SECONDS {
        return Verdict::Recheck;
    }
    Verdict::Available
}

fn blocked(blockers: Vec<Blocker>) -> Blocked {
    // `Option` folds the way the rule reads: the maximum, unless any blocker has no time.
    let expected_available_at = blockers
        .iter()
        .map(|blocker| blocker.recovers_at())
        .try_fold(i64::MIN, |latest, at| at.map(|at| latest.max(at)));
    Blocked {
        blockers,
        expected_available_at,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowState {
    HasRoom,
    Exhausted,
    Unknown,
}

fn window_blocker(snapshot: &QuotaSnapshot, kind: WindowKind) -> WindowState {
    let Some(window) = snapshot.quota().window(kind) else {
        return WindowState::Unknown;
    };
    if window.remaining_percent.is_nan() {
        return WindowState::Unknown;
    }
    if window.remaining_percent <= 0.0 {
        WindowState::Exhausted
    } else {
        WindowState::HasRoom
    }
}

/// One participating account, judged, with what the ordering rule needs to know about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub account_id: String,
    pub verdict: Verdict,
    /// Position in the participant list, which is the user's priority order.
    pub list_position: usize,
    /// Unix seconds; the final tie-breaker.
    pub created_at: i64,
}

/// Why an account was placed first. Shown as "will use: X (reason)".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The user ordered the participants and this one is the first that can run.
    UserPriority,
    /// It is running the task already, so no switch is needed.
    CurrentlyExecuting,
    /// Earliest in the participant list among those that can run.
    ListPosition,
    /// Same list position is impossible, so this only breaks a tie between equal positions
    /// in a list the caller built badly; kept so the rule is total.
    CreatedEarlier,
    /// Nobody can run now; this one has the earliest expected recovery.
    EarliestAvailable,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserPriority => "user_priority",
            Self::CurrentlyExecuting => "currently_executing",
            Self::ListPosition => "list_position",
            Self::CreatedEarlier => "created_earlier",
            Self::EarliestAvailable => "earliest_available",
        }
    }
}

/// What the scheduler should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Use this account now.
    Now { account_id: String, reason: Reason },
    /// Nobody is usable on current facts, but these readings are too old or missing; read
    /// them again before deciding anything.
    Recheck { account_ids: Vec<String> },
    /// Everybody is blocked; wait for this one, the earliest expected to recover. `blockers`
    /// is what it is waiting on, so the wait can be explained without a second lookup.
    Later {
        account_id: String,
        available_at: i64,
        blockers: Vec<Blocker>,
        reason: Reason,
    },
    /// Everybody is blocked and nobody was given a recovery time, but this one is blocked
    /// only by things that lift with time. Wait a while and look again; nothing here needs a
    /// person.
    LaterUnknown {
        account_id: String,
        blockers: Vec<Blocker>,
        reason: Reason,
    },
    /// Nobody is usable, and what blocks them does not lift on its own. A person has to
    /// decide.
    NeedsHuman,
}

/// Picks the account to use.
///
/// * Several available: the user's order when they set one, otherwise the account already
///   executing, then list position, then creation time.
/// * None available but some readings stale or missing: re-check those first.
/// * All blocked: the earliest expected recovery, ties broken by the same order.
/// * No expected time anywhere, but something blocking lifts with time: look again later.
/// * Nothing that lifts on its own: needs a human.
///
/// Deterministic: equal candidates keep the order they were given in.
pub fn choose(candidates: &[Candidate], executing: Option<&str>, user_ordered: bool) -> Selection {
    let mut ordered: Vec<&Candidate> = candidates.iter().collect();
    ordered.sort_by_key(|candidate| priority_key(candidate, executing, user_ordered));

    if let Some(first) = ordered
        .iter()
        .find(|candidate| candidate.verdict == Verdict::Available)
    {
        let reason = if user_ordered {
            Reason::UserPriority
        } else if executing == Some(first.account_id.as_str()) {
            Reason::CurrentlyExecuting
        } else if ordered.iter().any(|other| {
            other.list_position == first.list_position && other.account_id != first.account_id
        }) {
            Reason::CreatedEarlier
        } else {
            Reason::ListPosition
        };
        return Selection::Now {
            account_id: first.account_id.clone(),
            reason,
        };
    }

    let recheck: Vec<String> = ordered
        .iter()
        .filter(|candidate| match &candidate.verdict {
            Verdict::Recheck => true,
            Verdict::Blocked(blocked) => blocked.is_recheckable(),
            Verdict::Available => false,
        })
        .map(|candidate| candidate.account_id.clone())
        .collect();
    if !recheck.is_empty() {
        return Selection::Recheck {
            account_ids: recheck,
        };
    }

    // `min_by_key` keeps the first of equal keys, so the priority order above breaks ties.
    let earliest = ordered
        .iter()
        .filter_map(|candidate| match &candidate.verdict {
            Verdict::Blocked(Blocked {
                expected_available_at: Some(at),
                blockers,
            }) => Some((*at, *candidate, blockers)),
            _ => None,
        });
    match earliest.min_by_key(|(at, _, _)| *at) {
        Some((available_at, candidate, blockers)) => Selection::Later {
            account_id: candidate.account_id.clone(),
            available_at,
            blockers: blockers.clone(),
            reason: Reason::EarliestAvailable,
        },
        None => waitable(&ordered),
    }
}

/// The first account, in priority order, blocked only by waiting; `NeedsHuman` when there is none.
fn waitable(ordered: &[&Candidate]) -> Selection {
    ordered
        .iter()
        .find_map(|candidate| match &candidate.verdict {
            Verdict::Blocked(blocked) if blocked.lifts_on_its_own() => {
                Some(Selection::LaterUnknown {
                    account_id: candidate.account_id.clone(),
                    blockers: blocked.blockers.clone(),
                    reason: Reason::EarliestAvailable,
                })
            }
            _ => None,
        })
        .unwrap_or(Selection::NeedsHuman)
}

/// The default order: the executing account first (no switch), then list position, then
/// creation time. With a user-set order the list position alone decides.
fn priority_key(
    candidate: &Candidate,
    executing: Option<&str>,
    user_ordered: bool,
) -> (bool, usize, i64) {
    let is_executing = executing == Some(candidate.account_id.as_str());
    (
        user_ordered || !is_executing,
        candidate.list_position,
        candidate.created_at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_server::{RawRateLimits, RawWindow};
    use crate::quota::NormalisedQuota;

    const NOW: i64 = 1_789_200_000;
    /// 03:00 tonight and 09:00 tomorrow, relative to `NOW`, as unix seconds.
    const AT_0300: i64 = NOW + 3 * 3600;
    const AT_0900_TOMORROW: i64 = NOW + 27 * 3600;

    fn window(used: f64, minutes: i64, resets_at: Option<i64>) -> RawWindow {
        RawWindow {
            used_percent: used,
            window_duration_mins: Some(minutes),
            resets_at,
        }
    }

    fn snapshot(
        primary: Option<RawWindow>,
        secondary: Option<RawWindow>,
        fetched_at: i64,
    ) -> QuotaSnapshot {
        let quota = NormalisedQuota::from_raw(&RawRateLimits {
            primary,
            secondary,
            plan_type: Some("plus".to_owned()),
            credits: None,
            by_limit_id: Default::default(),
        });
        QuotaSnapshot::fresh("acct", quota, fetched_at)
    }

    fn both(five_hour_used: f64, weekly_used: f64) -> QuotaSnapshot {
        snapshot(
            Some(window(five_hour_used, 300, Some(AT_0300))),
            Some(window(weekly_used, 10_080, Some(AT_0900_TOMORROW))),
            NOW,
        )
    }

    fn facts(snapshot: &QuotaSnapshot) -> AccountFacts<'_> {
        AccountFacts {
            status: AccountStatus::Ready,
            quota: Some(snapshot),
            last_turn_error: None,
        }
    }

    fn blockers(verdict: &Verdict) -> Vec<Blocker> {
        match verdict {
            Verdict::Blocked(blocked) => blocked.blockers.clone(),
            other => panic!("expected a blocked verdict, got {other:?}"),
        }
    }

    fn expected_at(verdict: &Verdict) -> Option<i64> {
        match verdict {
            Verdict::Blocked(blocked) => blocked.expected_available_at,
            other => panic!("expected a blocked verdict, got {other:?}"),
        }
    }

    #[test]
    fn a_fresh_reading_with_room_in_both_windows_is_available() {
        let snapshot = both(40.0, 60.0);
        assert_eq!(assess(&facts(&snapshot), NOW), Verdict::Available);
    }

    // The expected time is the *latest* recovery, not the earliest.
    #[test]
    fn both_windows_exhausted_recover_when_the_weekly_one_does() {
        let snapshot = both(100.0, 100.0);

        let verdict = assess(&facts(&snapshot), NOW);

        assert_eq!(
            blockers(&verdict),
            [
                Blocker::FiveHourExhausted {
                    resets_at: Some(AT_0300)
                },
                Blocker::WeeklyExhausted {
                    resets_at: Some(AT_0900_TOMORROW)
                },
            ]
        );
        assert_eq!(
            expected_at(&verdict),
            Some(AT_0900_TOMORROW),
            "03:00 would be a lie: the weekly window is still empty then"
        );
    }

    // The five-hour window came back, the weekly one did not.
    #[test]
    fn a_recovered_five_hour_window_does_not_unblock_an_empty_weekly_one() {
        let snapshot = both(10.0, 100.0);

        let verdict = assess(&facts(&snapshot), NOW);

        assert_eq!(
            blockers(&verdict),
            [Blocker::WeeklyExhausted {
                resets_at: Some(AT_0900_TOMORROW)
            }]
        );
        assert_eq!(expected_at(&verdict), Some(AT_0900_TOMORROW));
    }

    #[test]
    fn a_window_without_a_reset_time_leaves_no_expected_time() {
        let snapshot = snapshot(
            Some(window(100.0, 300, None)),
            Some(window(100.0, 10_080, Some(AT_0900_TOMORROW))),
            NOW,
        );

        let verdict = assess(&facts(&snapshot), NOW);

        assert_eq!(
            expected_at(&verdict),
            None,
            "one blocker without a time means no time; the weekly reset is not a substitute"
        );
    }

    #[test]
    fn an_account_that_must_sign_in_again_is_blocked_for_good() {
        let snapshot = both(40.0, 60.0);
        let facts = AccountFacts {
            status: AccountStatus::ReauthRequired,
            ..facts(&snapshot)
        };

        let verdict = assess(&facts, NOW);

        assert_eq!(blockers(&verdict), [Blocker::ReauthRequired]);
        assert_eq!(expected_at(&verdict), None);
    }

    #[test]
    fn an_unsupported_account_is_blocked_for_good() {
        let snapshot = both(40.0, 60.0);
        let facts = AccountFacts {
            status: AccountStatus::Unsupported,
            ..facts(&snapshot)
        };
        assert_eq!(blockers(&assess(&facts, NOW)), [Blocker::Unsupported]);
    }

    #[test]
    fn no_reading_at_all_is_unknown_and_recheckable() {
        let facts = AccountFacts {
            status: AccountStatus::Ready,
            quota: None,
            last_turn_error: None,
        };

        let verdict = assess(&facts, NOW);

        assert_eq!(blockers(&verdict), [Blocker::QuotaUnknown]);
        assert_eq!(expected_at(&verdict), None);
        assert!(matches!(&verdict, Verdict::Blocked(b) if b.is_recheckable()));
    }

    // A window the server did not return is not 0% and not 100%.
    #[test]
    fn a_missing_weekly_window_is_unknown_not_exhausted_and_not_full() {
        let snapshot = snapshot(Some(window(10.0, 300, Some(AT_0300))), None, NOW);

        let verdict = assess(&facts(&snapshot), NOW);

        assert_eq!(blockers(&verdict), [Blocker::QuotaUnknown]);
    }

    #[test]
    fn a_reading_that_is_not_a_number_is_unknown() {
        let snapshot = snapshot(
            Some(window(f64::NAN, 300, Some(AT_0300))),
            Some(window(10.0, 10_080, Some(AT_0900_TOMORROW))),
            NOW,
        );
        assert_eq!(
            blockers(&assess(&facts(&snapshot), NOW)),
            [Blocker::QuotaUnknown]
        );
    }

    #[test]
    fn a_stale_reading_with_room_asks_for_a_recheck_rather_than_a_start() {
        let snapshot = both_at(40.0, 60.0, NOW - STALE_AFTER_SECONDS - 1);
        assert_eq!(assess(&facts(&snapshot), NOW), Verdict::Recheck);

        let just_fresh = both_at(40.0, 60.0, NOW - STALE_AFTER_SECONDS);
        assert_eq!(assess(&facts(&just_fresh), NOW), Verdict::Available);
    }

    fn both_at(five_hour_used: f64, weekly_used: f64, fetched_at: i64) -> QuotaSnapshot {
        snapshot(
            Some(window(five_hour_used, 300, Some(AT_0300))),
            Some(window(weekly_used, 10_080, Some(AT_0900_TOMORROW))),
            fetched_at,
        )
    }

    #[test]
    fn a_stale_reading_that_shows_exhaustion_is_still_a_blocker_with_its_time() {
        let snapshot = both_at(100.0, 10.0, NOW - 2 * STALE_AFTER_SECONDS);

        let verdict = assess(&facts(&snapshot), NOW);

        assert_eq!(expected_at(&verdict), Some(AT_0300));
    }

    // The turn failed on a limit the percentages do not show.
    #[test]
    fn a_usage_limit_failure_with_room_in_the_numbers_is_a_server_limit() {
        let snapshot = both(40.0, 60.0);
        let facts = AccountFacts {
            last_turn_error: Some(TurnErrorKind::UsageLimitExceeded),
            ..facts(&snapshot)
        };

        let verdict = assess(&facts, NOW);

        assert_eq!(
            blockers(&verdict),
            [Blocker::ServerLimited {
                resets_at: Some(AT_0300)
            }]
        );
        assert_eq!(expected_at(&verdict), Some(AT_0300));
    }

    #[test]
    fn a_usage_limit_failure_that_the_numbers_confirm_is_not_double_counted() {
        let snapshot = both(100.0, 60.0);
        let facts = AccountFacts {
            last_turn_error: Some(TurnErrorKind::UsageLimitExceeded),
            ..facts(&snapshot)
        };
        assert_eq!(
            blockers(&assess(&facts, NOW)),
            [Blocker::FiveHourExhausted {
                resets_at: Some(AT_0300)
            }]
        );
    }

    #[test]
    fn other_turn_failures_do_not_block_availability() {
        let snapshot = both(40.0, 60.0);
        for error in [
            TurnErrorKind::Network { http_status: None },
            TurnErrorKind::Unauthorized,
            TurnErrorKind::RateLimitExceeded,
            TurnErrorKind::Other,
        ] {
            let facts = AccountFacts {
                last_turn_error: Some(error),
                ..facts(&snapshot)
            };
            assert_eq!(assess(&facts, NOW), Verdict::Available, "{error:?}");
        }
    }

    // ---- choosing ----

    fn candidate(id: &str, verdict: Verdict, position: usize, created_at: i64) -> Candidate {
        Candidate {
            account_id: id.to_owned(),
            verdict,
            list_position: position,
            created_at,
        }
    }

    fn blocked_until(at: Option<i64>) -> Verdict {
        Verdict::Blocked(Blocked {
            blockers: vec![Blocker::FiveHourExhausted { resets_at: at }],
            expected_available_at: at,
        })
    }

    fn blocked_for_good() -> Verdict {
        Verdict::Blocked(Blocked {
            blockers: vec![Blocker::ReauthRequired],
            expected_available_at: None,
        })
    }

    #[test]
    fn the_executing_account_is_preferred_when_the_user_set_no_order() {
        let candidates = [
            candidate("a", Verdict::Available, 0, 100),
            candidate("b", Verdict::Available, 1, 200),
        ];

        let selection = choose(&candidates, Some("b"), false);

        assert_eq!(
            selection,
            Selection::Now {
                account_id: "b".to_owned(),
                reason: Reason::CurrentlyExecuting
            }
        );
    }

    #[test]
    fn a_user_set_order_outranks_the_executing_account() {
        let candidates = [
            candidate("a", Verdict::Available, 0, 100),
            candidate("b", Verdict::Available, 1, 200),
        ];

        let selection = choose(&candidates, Some("b"), true);

        assert_eq!(
            selection,
            Selection::Now {
                account_id: "a".to_owned(),
                reason: Reason::UserPriority
            }
        );
    }

    #[test]
    fn list_position_then_creation_time_break_ties() {
        let candidates = [
            candidate("late", Verdict::Available, 1, 300),
            candidate("early", Verdict::Available, 1, 100),
            candidate("blocked", blocked_until(Some(AT_0300)), 0, 1),
        ];

        let selection = choose(&candidates, None, false);

        assert_eq!(
            selection,
            Selection::Now {
                account_id: "early".to_owned(),
                reason: Reason::CreatedEarlier
            }
        );

        let by_position = [
            candidate("second", Verdict::Available, 1, 100),
            candidate("first", Verdict::Available, 0, 900),
        ];
        assert_eq!(
            choose(&by_position, None, false),
            Selection::Now {
                account_id: "first".to_owned(),
                reason: Reason::ListPosition
            }
        );
    }

    // At the selection level: the account that is *really* earliest wins.
    #[test]
    fn when_everybody_is_blocked_the_genuinely_earliest_recovery_wins() {
        let candidates = [
            // Looks earliest (five-hour at 03:00) but its weekly window holds it to tomorrow.
            candidate(
                "misleading",
                Verdict::Blocked(Blocked {
                    blockers: vec![
                        Blocker::FiveHourExhausted {
                            resets_at: Some(AT_0300),
                        },
                        Blocker::WeeklyExhausted {
                            resets_at: Some(AT_0900_TOMORROW),
                        },
                    ],
                    expected_available_at: Some(AT_0900_TOMORROW),
                }),
                0,
                1,
            ),
            candidate("later_tonight", blocked_until(Some(NOW + 5 * 3600)), 1, 2),
            candidate("never", blocked_for_good(), 2, 3),
        ];

        let selection = choose(&candidates, Some("misleading"), false);

        assert_eq!(
            selection,
            Selection::Later {
                account_id: "later_tonight".to_owned(),
                available_at: NOW + 5 * 3600,
                blockers: vec![Blocker::FiveHourExhausted {
                    resets_at: Some(NOW + 5 * 3600)
                }],
                reason: Reason::EarliestAvailable
            }
        );
    }

    #[test]
    fn equal_recovery_times_fall_back_to_the_priority_order() {
        let candidates = [
            candidate("a", blocked_until(Some(AT_0300)), 1, 1),
            candidate("b", blocked_until(Some(AT_0300)), 0, 2),
        ];
        assert!(matches!(
            choose(&candidates, None, false),
            Selection::Later { account_id, .. } if account_id == "b"
        ));
        assert!(matches!(
            choose(&candidates, Some("a"), false),
            Selection::Later { account_id, .. } if account_id == "a"
        ));
    }

    // Only a person can renew a sign-in, so a list of nothing else stops.
    #[test]
    fn when_nothing_lifts_on_its_own_a_person_is_needed() {
        let candidates = [
            candidate("a", blocked_for_good(), 0, 1),
            candidate("b", blocked_for_good(), 1, 2),
        ];
        assert_eq!(choose(&candidates, None, false), Selection::NeedsHuman);
    }

    /// An exhausted window whose reset time the server withheld is still an exhausted window:
    /// it reopens on its own. Looking again later is the answer, not a button.
    #[test]
    fn an_exhausted_window_with_no_reset_time_is_waited_for_not_handed_over() {
        let candidates = [
            candidate("a", blocked_for_good(), 0, 1),
            candidate("b", blocked_until(None), 1, 2),
        ];
        assert!(matches!(
            choose(&candidates, None, false),
            Selection::LaterUnknown { account_id, blockers, .. }
                if account_id == "b" && blockers == vec![Blocker::FiveHourExhausted { resets_at: None }]
        ));
    }

    #[test]
    fn a_reset_time_beats_no_reset_time() {
        let candidates = [
            candidate("no_time", blocked_until(None), 0, 1),
            candidate("at_three", blocked_until(Some(AT_0300)), 1, 2),
        ];
        assert!(matches!(
            choose(&candidates, None, false),
            Selection::Later { account_id, .. } if account_id == "at_three"
        ));
    }

    #[test]
    fn every_blocker_says_whether_waiting_is_enough() {
        for blocker in [
            Blocker::FiveHourExhausted { resets_at: None },
            Blocker::WeeklyExhausted { resets_at: None },
            Blocker::ServerLimited { resets_at: None },
            Blocker::QuotaUnknown,
        ] {
            assert!(blocker.lifts_on_its_own(), "{blocker:?}");
        }
        for blocker in [Blocker::ReauthRequired, Blocker::Unsupported] {
            assert!(!blocker.lifts_on_its_own(), "{blocker:?}");
        }
    }

    /// One blocker a person has to clear is enough to make the whole account a person's job,
    /// even beside one that would have lifted by itself.
    #[test]
    fn an_account_mixing_the_two_kinds_needs_a_person() {
        let mixed = Verdict::Blocked(Blocked {
            blockers: vec![
                Blocker::FiveHourExhausted { resets_at: None },
                Blocker::ReauthRequired,
            ],
            expected_available_at: None,
        });
        assert_eq!(
            choose(&[candidate("a", mixed, 0, 1)], None, false),
            Selection::NeedsHuman
        );
    }

    #[test]
    fn stale_or_unknown_readings_are_rechecked_before_anyone_waits() {
        let candidates = [
            candidate("waiting", blocked_until(Some(AT_0300)), 0, 1),
            candidate("stale", Verdict::Recheck, 1, 2),
            candidate(
                "unknown",
                Verdict::Blocked(Blocked {
                    blockers: vec![Blocker::QuotaUnknown],
                    expected_available_at: None,
                }),
                2,
                3,
            ),
        ];

        assert_eq!(
            choose(&candidates, None, false),
            Selection::Recheck {
                account_ids: vec!["stale".to_owned(), "unknown".to_owned()]
            }
        );
    }

    #[test]
    fn an_available_account_is_used_even_when_others_need_a_recheck() {
        let candidates = [
            candidate("stale", Verdict::Recheck, 0, 1),
            candidate("ready", Verdict::Available, 1, 2),
        ];
        assert!(matches!(
            choose(&candidates, None, false),
            Selection::Now { account_id, .. } if account_id == "ready"
        ));
    }

    #[test]
    fn no_candidates_at_all_needs_a_person() {
        assert_eq!(choose(&[], None, false), Selection::NeedsHuman);
    }

    #[test]
    fn every_blocker_and_reason_has_a_distinct_stable_name() {
        let blockers = [
            Blocker::ReauthRequired,
            Blocker::Unsupported,
            Blocker::FiveHourExhausted { resets_at: None },
            Blocker::WeeklyExhausted { resets_at: None },
            Blocker::ServerLimited { resets_at: None },
            Blocker::QuotaUnknown,
        ];
        let names: std::collections::BTreeSet<_> = blockers.iter().map(|b| b.as_str()).collect();
        assert_eq!(names.len(), blockers.len());

        let reasons = [
            Reason::UserPriority,
            Reason::CurrentlyExecuting,
            Reason::ListPosition,
            Reason::CreatedEarlier,
            Reason::EarliestAvailable,
        ];
        let names: std::collections::BTreeSet<_> = reasons.iter().map(|r| r.as_str()).collect();
        assert_eq!(names.len(), reasons.len());
    }
}
