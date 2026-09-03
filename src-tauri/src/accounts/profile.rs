//! The stored, non-sensitive record of one account.

use serde::{Deserialize, Serialize};

use super::status::AccountStatus;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Display name bounds. Counted in characters, not bytes: a 24-character Chinese
/// name is 24 characters to the person who typed it.
const NAME_MIN_CHARS: usize = 1;
const NAME_MAX_CHARS: usize = 24;

/// One account as Toglet stores it.
///
/// **There is no token field and there never may be.** The credential lives in the platform
/// store; `credential_ref` is only a key into it. Adding a field here that could
/// hold credential material is the failure this type's shape exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    /// Random internal id. **This is the only account identifier that may appear in a log** -
    /// not the fingerprint, and never the address.
    pub id: String,
    pub display_name: String,
    /// Masked form, such as `lea***@gmail.com`. The full address is never stored here.
    pub masked_email: Option<String>,
    /// Irreversible derivation of the stable identifier. Used for deduplication only.
    pub account_fingerprint: String,
    /// `None` when the plan is unknown. Never guessed, never defaulted to a plan name.
    pub plan_type: Option<String>,
    /// MVP manages ChatGPT sign-ins only.
    pub auth_mode: String,
    /// Key into the credential store. Carries no credential material itself.
    pub credential_ref: String,
    pub status: AccountStatus,
    pub created_at: String,
    pub updated_at: String,
    pub last_validated_at: Option<String>,
}

/// Trims and validates a display name.
///
/// Rejects: empty, whitespace-only, and anything longer than 24 characters. Accepts everything
/// else including emoji - a name is user data, and there is no security reason to narrow it
/// further. A name never reaches a command line or an environment variable; that is enforced
/// by `app_server::process` having a constant argument list, not by filtering here.
pub fn validate_display_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let length = trimmed.chars().count();

    if (NAME_MIN_CHARS..=NAME_MAX_CHARS).contains(&length) {
        Ok(trimmed.to_owned())
    } else {
        Err(
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail("display name must be 1 to 24 characters after trimming"),
        )
    }
}

/// The name an account gets when the user has not chosen one.
///
/// The account's own name first - the one ChatGPT requires at sign-up and carries in the id
/// token - and the local part of the address when there is none. Cut to the profile's limit
/// rather than refused: a default that failed validation would put the user back at a naming
/// step the flow no longer has.
pub fn default_display_name(nickname: Option<&str>, email: Option<&str>) -> Option<String> {
    let nickname = nickname.map(str::trim).filter(|name| !name.is_empty());
    let local_part = email
        .and_then(|address| address.split_once('@'))
        .map(|(local, _)| local.trim())
        .filter(|local| !local.is_empty());
    nickname
        .or(local_part)
        .map(|name| name.chars().take(NAME_MAX_CHARS).collect())
}

/// Masks an address for display and storage: `leanne@gmail.com` becomes `lea***@gmail.com`.
///
/// Short local parts are masked entirely rather than partially - `ab@x.com` must not become
/// `ab***@x.com`, which would disclose the whole local part.
pub fn mask_email(address: &str) -> Option<String> {
    let (local, domain) = address.split_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }

    let visible: String = if local.chars().count() > 4 {
        local.chars().take(3).collect()
    } else {
        String::new()
    };
    Some(format!("{visible}***@{domain}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_name_prefers_the_accounts_own_name() {
        assert_eq!(
            default_display_name(Some("Leanne Q"), Some("leanne@example.com")).as_deref(),
            Some("Leanne Q")
        );
    }

    #[test]
    fn the_default_name_falls_back_to_the_local_part_of_the_address() {
        assert_eq!(
            default_display_name(None, Some("leanne@example.com")).as_deref(),
            Some("leanne")
        );
        assert_eq!(
            default_display_name(Some("  "), Some("leanne@example.com")).as_deref(),
            Some("leanne")
        );
    }

    #[test]
    fn the_default_name_is_cut_to_the_limit_rather_than_refused() {
        let long = "x".repeat(NAME_MAX_CHARS + 10);

        let name = default_display_name(Some(&long), None).expect("a name");

        assert_eq!(name.chars().count(), NAME_MAX_CHARS);
        assert!(validate_display_name(&name).is_ok());
    }

    #[test]
    fn no_name_and_no_address_gives_no_default() {
        assert_eq!(default_display_name(None, None), None);
        assert_eq!(default_display_name(None, Some("@example.com")), None);
    }

    #[test]
    fn a_name_is_accepted_at_both_bounds() {
        assert_eq!(validate_display_name("a").expect("1 char"), "a");
        let longest = "x".repeat(NAME_MAX_CHARS);
        assert_eq!(validate_display_name(&longest).expect("24 chars"), longest);
    }

    #[test]
    fn a_name_is_rejected_just_outside_both_bounds() {
        assert!(validate_display_name("").is_err());
        assert!(validate_display_name(&"x".repeat(NAME_MAX_CHARS + 1)).is_err());
    }

    #[test]
    fn whitespace_only_is_rejected_and_padding_is_trimmed() {
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name("\t\n ").is_err());
        assert_eq!(validate_display_name("  work  ").expect("trimmed"), "work");
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        // 24 Chinese characters are 72 bytes; the limit is about what the user sees.
        let chinese = "工".repeat(NAME_MAX_CHARS);
        assert_eq!(
            validate_display_name(&chinese).expect("24 characters"),
            chinese
        );
        assert!(validate_display_name(&"工".repeat(NAME_MAX_CHARS + 1)).is_err());
    }

    #[test]
    fn emoji_are_accepted_because_a_name_is_user_data() {
        assert_eq!(
            validate_display_name("work 🔵").expect("accepted"),
            "work 🔵"
        );
    }

    #[test]
    fn masking_keeps_the_domain_and_three_characters_of_a_long_local_part() {
        assert_eq!(
            mask_email("leanne@gmail.com").as_deref(),
            Some("lea***@gmail.com")
        );
    }

    #[test]
    fn a_short_local_part_is_masked_entirely() {
        // Keeping three of four characters would disclose almost the whole local part.
        assert_eq!(mask_email("ab@x.com").as_deref(), Some("***@x.com"));
        assert_eq!(mask_email("abcd@x.com").as_deref(), Some("***@x.com"));
    }

    #[test]
    fn something_that_is_not_an_address_masks_to_nothing() {
        assert_eq!(mask_email("no-at-sign"), None);
        assert_eq!(mask_email("@domain.com"), None);
        assert_eq!(mask_email("local@"), None);
    }

    #[test]
    fn a_masked_address_never_contains_the_full_local_part() {
        for address in ["leanne@gmail.com", "a@b.co", "verylongname@example.org"] {
            let (local, _) = address.split_once('@').expect("an address");
            let masked = mask_email(address).expect("masked");
            assert!(
                !masked.contains(local),
                "{masked} still contains the local part of {address}"
            );
        }
    }
}
