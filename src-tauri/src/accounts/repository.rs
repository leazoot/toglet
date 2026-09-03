//! Account profile CRUD over the metadata document.

use super::fingerprint::{DuplicateCheck, check_duplicate};
use super::profile::{AccountProfile, validate_display_name};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::storage::MetadataDocument;

/// Hard limit on stored accounts.
///
/// Twelve. A limit exists at all because every account is a credential entry to keep and a row
/// the expanded panel has to show; the number itself is a product choice, not a technical
/// ceiling.
pub const MAX_ACCOUNTS: usize = 12;

/// Adds a profile, refusing duplicates and refusing to exceed the limit.
///
/// A duplicate is not an error the user has to work around: the caller is handed the existing
/// account's id so it can offer to refresh that account's credentials instead.
pub fn add(document: &mut MetadataDocument, profile: AccountProfile) -> Result<DuplicateCheck> {
    let existing = document
        .accounts
        .iter()
        .map(|account| (account.id.as_str(), account.account_fingerprint.as_str()));

    if let DuplicateCheck::AlreadyPresent { existing_id } =
        check_duplicate(&profile.account_fingerprint, existing)
    {
        return Ok(DuplicateCheck::AlreadyPresent { existing_id });
    }

    if document.accounts.len() >= MAX_ACCOUNTS {
        return Err(
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail("the account limit has been reached"),
        );
    }

    document.accounts.push(profile);
    Ok(DuplicateCheck::New)
}

pub fn find<'a>(document: &'a MetadataDocument, id: &str) -> Option<&'a AccountProfile> {
    document.accounts.iter().find(|account| account.id == id)
}

/// Renames an account after validating the new name.
pub fn rename(document: &mut MetadataDocument, id: &str, name: &str) -> Result<()> {
    let validated = validate_display_name(name)?;
    let account = document
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or_else(not_found)?;
    account.display_name = validated;
    Ok(())
}

/// Removes an account.
///
/// Refuses to remove the currently active one: that would leave Codex signed in as an account
/// Toglet no longer knows about. The caller switches away first, or takes the explicit
/// sign-out path.
pub fn remove(document: &mut MetadataDocument, id: &str) -> Result<AccountProfile> {
    if document.settings.active_account_id() == Some(id) {
        return Err(TogletError::new(
            ErrorCode::Internal,
            Phase::Storage,
            false,
            UserAction::WaitForSwitch,
        )
        .with_detail("the active account cannot be removed without switching away first"));
    }

    let index = document
        .accounts
        .iter()
        .position(|account| account.id == id)
        .ok_or_else(not_found)?;
    Ok(document.accounts.remove(index))
}

fn not_found() -> TogletError {
    TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
        .with_detail("no such account")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::AccountStatus;
    use crate::storage::SwitchVerified;

    fn profile(id: &str, fingerprint: &str) -> AccountProfile {
        AccountProfile {
            id: id.to_owned(),
            display_name: "Work".to_owned(),
            masked_email: Some("lea***@gmail.com".to_owned()),
            account_fingerprint: fingerprint.to_owned(),
            plan_type: None,
            auth_mode: "chatgpt".to_owned(),
            credential_ref: format!("cred-{id}"),
            status: AccountStatus::Ready,
            created_at: "2026-08-31T00:00:00Z".to_owned(),
            updated_at: "2026-08-31T00:00:00Z".to_owned(),
            last_validated_at: None,
        }
    }

    #[test]
    fn adding_a_new_account_stores_it() {
        let mut document = MetadataDocument::default();

        let outcome = add(&mut document, profile("acct-1", "fp-1")).expect("added");

        assert_eq!(outcome, DuplicateCheck::New);
        assert_eq!(document.accounts.len(), 1);
    }

    #[test]
    fn adding_the_same_account_twice_reports_the_existing_one_and_adds_nothing() {
        let mut document = MetadataDocument::default();
        add(&mut document, profile("acct-1", "fp-1")).expect("added");

        let outcome = add(&mut document, profile("acct-2", "fp-1")).expect("checked");

        assert_eq!(
            outcome,
            DuplicateCheck::AlreadyPresent {
                existing_id: "acct-1".to_owned()
            }
        );
        assert_eq!(document.accounts.len(), 1, "no second profile was created");
    }

    #[test]
    fn the_account_limit_is_enforced_at_twelve() {
        let mut document = MetadataDocument::default();
        for index in 0..MAX_ACCOUNTS {
            add(
                &mut document,
                profile(&format!("acct-{index}"), &format!("fp-{index}")),
            )
            .expect("within the limit");
        }

        let error = add(&mut document, profile("acct-extra", "fp-extra"))
            .expect_err("the limit is a hard limit");

        assert_eq!(error.code(), ErrorCode::Internal);
        assert_eq!(document.accounts.len(), MAX_ACCOUNTS);
    }

    #[test]
    fn renaming_validates_the_new_name() {
        let mut document = MetadataDocument::default();
        add(&mut document, profile("acct-1", "fp-1")).expect("added");

        assert!(rename(&mut document, "acct-1", "   ").is_err());
        assert!(rename(&mut document, "acct-1", &"x".repeat(25)).is_err());
        rename(&mut document, "acct-1", "  Personal  ").expect("valid name");

        assert_eq!(
            find(&document, "acct-1").expect("present").display_name,
            "Personal"
        );
    }

    #[test]
    fn renaming_an_unknown_account_is_an_error() {
        let mut document = MetadataDocument::default();

        assert!(rename(&mut document, "absent", "Name").is_err());
    }

    #[test]
    fn removing_a_stored_account_returns_it() {
        let mut document = MetadataDocument::default();
        add(&mut document, profile("acct-1", "fp-1")).expect("added");
        add(&mut document, profile("acct-2", "fp-2")).expect("added");

        let removed = remove(&mut document, "acct-1").expect("removed");

        assert_eq!(removed.id, "acct-1");
        assert_eq!(document.accounts.len(), 1);
        assert!(find(&document, "acct-1").is_none());
    }

    #[test]
    fn the_active_account_cannot_be_removed_while_it_is_active() {
        let mut document = MetadataDocument::default();
        add(&mut document, profile("acct-1", "fp-1")).expect("added");
        document
            .settings
            .set_active_account_id(Some("acct-1".to_owned()), &SwitchVerified::issue());

        let error = remove(&mut document, "acct-1").expect_err("removal is refused");

        assert_eq!(error.action(), UserAction::WaitForSwitch);
        assert_eq!(document.accounts.len(), 1, "nothing was removed");
    }

    #[test]
    fn removing_an_unknown_account_is_an_error_rather_than_a_silent_success() {
        let mut document = MetadataDocument::default();

        assert!(remove(&mut document, "absent").is_err());
    }
}
