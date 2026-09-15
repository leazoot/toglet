//! Recognising account types Toglet cannot manage.
//!
//! An unmanageable account is kept out of the switch list and its quota is unknown (`null`),
//! never a fabricated `0`.

use super::status::AccountStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountKind {
    /// The only kind Toglet manages.
    Chatgpt,
    ApiKey,
    /// `chatgptAuthTokens`: in-memory tokens from a host application (the app server schema marks
    /// it "FOR OPENAI INTERNAL USE ONLY"), so there is nothing on disk to store or replace.
    HostManagedTokens,
    /// A mode this build does not know, reported as-is rather than guessed.
    Unknown(String),
}

/// Why an account cannot be managed: a stable code the frontend maps to localised copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    ApiKeyHasNoQuotaWindows,
    TokensHeldByHostApplication,
    UnrecognisedAuthMode,
}

impl UnsupportedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKeyHasNoQuotaWindows => "api_key_has_no_quota_windows",
            Self::TokensHeldByHostApplication => "tokens_held_by_host_application",
            Self::UnrecognisedAuthMode => "unrecognised_auth_mode",
        }
    }
}

impl AccountKind {
    /// Maps the `auth_mode` field of an `auth.json`; the caller handles a missing field (signed out).
    pub fn from_auth_mode(mode: &str) -> Self {
        match mode {
            "chatgpt" => Self::Chatgpt,
            "apikey" => Self::ApiKey,
            "chatgptAuthTokens" => Self::HostManagedTokens,
            other => Self::Unknown(other.to_owned()),
        }
    }

    pub fn is_manageable(&self) -> bool {
        matches!(self, Self::Chatgpt)
    }

    /// `None` for a manageable account.
    pub fn unsupported_reason(&self) -> Option<UnsupportedReason> {
        match self {
            Self::Chatgpt => None,
            Self::ApiKey => Some(UnsupportedReason::ApiKeyHasNoQuotaWindows),
            Self::HostManagedTokens => Some(UnsupportedReason::TokensHeldByHostApplication),
            Self::Unknown(_) => Some(UnsupportedReason::UnrecognisedAuthMode),
        }
    }

    pub fn status(&self) -> AccountStatus {
        if self.is_manageable() {
            AccountStatus::Ready
        } else {
            AccountStatus::Unsupported
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_chatgpt_sign_in_is_manageable() {
        assert!(AccountKind::from_auth_mode("chatgpt").is_manageable());
        for other in ["apikey", "chatgptAuthTokens", "somethingNew"] {
            assert!(
                !AccountKind::from_auth_mode(other).is_manageable(),
                "{other} must not be manageable"
            );
        }
    }

    #[test]
    fn an_unmanageable_account_is_recorded_as_unsupported_not_as_an_error() {
        // Not `error`: nothing failed, the account is simply not one Toglet manages.
        assert_eq!(
            AccountKind::from_auth_mode("apikey").status(),
            AccountStatus::Unsupported
        );
        assert!(!AccountStatus::Unsupported.may_start_switch());
    }

    #[test]
    fn every_unmanageable_kind_carries_a_distinct_reason() {
        let reasons: Vec<&str> = ["apikey", "chatgptAuthTokens", "whatever"]
            .into_iter()
            .map(|mode| {
                AccountKind::from_auth_mode(mode)
                    .unsupported_reason()
                    .expect("unmanageable kinds carry a reason")
                    .as_str()
            })
            .collect();

        let unique: std::collections::BTreeSet<_> = reasons.iter().collect();
        assert_eq!(
            unique.len(),
            reasons.len(),
            "reasons must be distinguishable"
        );
    }

    #[test]
    fn a_manageable_account_has_no_reason_to_explain() {
        assert_eq!(
            AccountKind::from_auth_mode("chatgpt").unsupported_reason(),
            None
        );
        assert_eq!(
            AccountKind::from_auth_mode("chatgpt").status(),
            AccountStatus::Ready
        );
    }

    #[test]
    fn an_unknown_mode_keeps_its_name_rather_than_being_folded_into_a_known_one() {
        assert_eq!(
            AccountKind::from_auth_mode("futureMode"),
            AccountKind::Unknown("futureMode".to_owned())
        );
    }
}
