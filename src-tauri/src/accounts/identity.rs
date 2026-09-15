//! What the app server says about the account a Codex home is signed in as.
//!
//! Must never gain a token field. `account/read` returns no stable id, only an e-mail address,
//! so this answers "who is signed in" for post-switch verification.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountIdentity {
    /// The only kind Toglet manages.
    Chatgpt {
        /// Full address as reported; masked before it leaves the Rust layer.
        email: String,
        /// `None` when unknown, including the server's own `"unknown"`.
        plan_type: Option<String>,
    },
    /// Recognised so it can be reported, then refused: API keys have no five-hour or weekly quota.
    ApiKey,
}

impl AccountIdentity {
    pub fn email(&self) -> Option<&str> {
        match self {
            Self::Chatgpt { email, .. } => Some(email),
            Self::ApiKey => None,
        }
    }

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
