//! What the app server can say about the account a Codex home is signed in as.
//!
//! There is deliberately no token field, and there never may be: the type system is the first
//! place a credential leak gets caught.
//!
//! `account/read` returns no stable account identifier - only an e-mail address. So this type
//! answers "who is signed in", which is what post-switch verification compares, while the
//! irreversible `accountFingerprint` used for deduplication is derived from the credential
//! material separately.

/// The identity a Codex home is currently signed in as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountIdentity {
    /// Signed in with a ChatGPT account - the only kind Toglet manages.
    Chatgpt {
        /// The full address as the server reported it. Only ever masked before it leaves the
        /// Rust layer; `maskedEmail` is what gets persisted.
        email: String,
        /// `None` when the plan is unknown. The server's own `"unknown"` maps here too: an
        /// unknown plan is stored as `null`, never guessed.
        plan_type: Option<String>,
    },
    /// Signed in with an API key. Recognised so it can be reported honestly, then refused:
    /// API key accounts do not use the five-hour and weekly quota system.
    ApiKey,
}

impl AccountIdentity {
    /// The e-mail address, when this kind of account has one.
    pub fn email(&self) -> Option<&str> {
        match self {
            Self::Chatgpt { email, .. } => Some(email),
            Self::ApiKey => None,
        }
    }

    /// Whether Toglet can manage and switch to this account.
    pub fn is_manageable(&self) -> bool {
        matches!(self, Self::Chatgpt { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_accounts_are_recognised_but_not_manageable() {
        let account = AccountIdentity::ApiKey;

        assert!(!account.is_manageable());
        assert_eq!(account.email(), None);
    }

    #[test]
    fn a_chatgpt_account_without_a_known_plan_reports_none_not_a_placeholder() {
        let account = AccountIdentity::Chatgpt {
            email: "someone@example.com".to_owned(),
            plan_type: None,
        };

        assert!(account.is_manageable());
        assert_eq!(account.email(), Some("someone@example.com"));
    }
}
