//! The account state machine.
//!
//! One implementation, shared by `quota` and `switching`. Each module keeping its own notion of
//! "what state is this account in" is how two parts of an app end up disagreeing about whether
//! a switch is allowed.

use serde::{Deserialize, Serialize};

/// The nine states an account can be in.
///
/// A single enum rather than a set of booleans: `refreshing` and `reauth_required` are mutually
/// exclusive, and a combination of flags can represent nonsense that this cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    /// Usable, not the current account.
    Ready,
    /// The account Codex is currently signed in as.
    Active,
    /// A quota read is in flight. Only the refresh indicator changes, never the panel.
    Refreshing,
    /// The cached quota is older than the freshness window. Values are still shown, marked.
    Stale,
    /// The last read failed for a network reason. Cached values stay visible.
    Offline,
    /// Authentication is no longer valid. Switching to this account is refused.
    ReauthRequired,
    /// Recognised but outside what Toglet manages, such as an API key sign-in.
    Unsupported,
    /// A switch to this account is running.
    Switching,
    /// A failure that is none of the above.
    Error,
}

impl AccountStatus {
    /// Stable wire form for the frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Refreshing => "refreshing",
            Self::Stale => "stale",
            Self::Offline => "offline",
            Self::ReauthRequired => "reauth_required",
            Self::Unsupported => "unsupported",
            Self::Switching => "switching",
            Self::Error => "error",
        }
    }

    /// Whether a switch to this account may start.
    ///
    /// `reauth_required` and `unsupported` are refused outright: starting a switch that cannot
    /// succeed would replace working credentials with broken ones.
    pub fn may_start_switch(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Stale | Self::Offline | Self::Error
        )
    }

    /// Whether `next` is a legal move from `self`.
    pub fn can_move_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            // Nothing may leave these two except an explicit re-login or re-detection, which
            // both go through `Ready`.
            Self::ReauthRequired | Self::Unsupported => next == Self::Ready,
            // A switch in flight ends in exactly one of three ways.
            Self::Switching => matches!(next, Self::Active | Self::Ready | Self::Error),
            // Everything else may move to any state that is not mid-switch, plus `switching`
            // only when a switch is actually permitted from here.
            _ => {
                if next == Self::Switching {
                    self.may_start_switch()
                } else {
                    true
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [AccountStatus; 9] = [
        AccountStatus::Ready,
        AccountStatus::Active,
        AccountStatus::Refreshing,
        AccountStatus::Stale,
        AccountStatus::Offline,
        AccountStatus::ReauthRequired,
        AccountStatus::Unsupported,
        AccountStatus::Switching,
        AccountStatus::Error,
    ];

    #[test]
    fn all_nine_states_have_a_distinct_wire_form() {
        let mut seen = std::collections::BTreeSet::new();
        for status in ALL {
            assert!(
                seen.insert(status.as_str()),
                "duplicate {}",
                status.as_str()
            );
        }
        assert_eq!(seen.len(), 9, "nine states are defined");
    }

    #[test]
    fn an_account_needing_re_login_cannot_be_switched_to() {
        assert!(!AccountStatus::ReauthRequired.may_start_switch());
        assert!(
            !AccountStatus::ReauthRequired.can_move_to(AccountStatus::Switching),
            "switching to a broken account would replace working credentials"
        );
    }

    #[test]
    fn an_unsupported_account_cannot_be_switched_to() {
        assert!(!AccountStatus::Unsupported.may_start_switch());
        assert!(!AccountStatus::Unsupported.can_move_to(AccountStatus::Switching));
    }

    #[test]
    fn a_broken_account_returns_to_ready_only() {
        for blocked in [AccountStatus::ReauthRequired, AccountStatus::Unsupported] {
            for next in ALL {
                let allowed = next == blocked || next == AccountStatus::Ready;
                assert_eq!(
                    blocked.can_move_to(next),
                    allowed,
                    "{} -> {}",
                    blocked.as_str(),
                    next.as_str()
                );
            }
        }
    }

    #[test]
    fn a_switch_in_flight_ends_in_exactly_three_ways() {
        for next in ALL {
            let allowed = matches!(
                next,
                AccountStatus::Active
                    | AccountStatus::Ready
                    | AccountStatus::Error
                    | AccountStatus::Switching
            );
            assert_eq!(
                AccountStatus::Switching.can_move_to(next),
                allowed,
                "switching -> {}",
                next.as_str()
            );
        }
    }

    #[test]
    fn a_usable_account_may_begin_switching() {
        for usable in [
            AccountStatus::Ready,
            AccountStatus::Stale,
            AccountStatus::Offline,
            AccountStatus::Error,
        ] {
            assert!(usable.may_start_switch(), "{}", usable.as_str());
            assert!(usable.can_move_to(AccountStatus::Switching));
        }
    }

    #[test]
    fn the_active_account_does_not_start_a_switch_to_itself() {
        assert!(!AccountStatus::Active.may_start_switch());
        assert!(!AccountStatus::Active.can_move_to(AccountStatus::Switching));
    }

    #[test]
    fn a_refresh_in_flight_does_not_begin_a_switch() {
        assert!(!AccountStatus::Refreshing.may_start_switch());
    }

    #[test]
    fn every_state_can_stay_where_it_is() {
        for status in ALL {
            assert!(status.can_move_to(status));
        }
    }
}
