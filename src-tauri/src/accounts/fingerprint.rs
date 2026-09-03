//! Irreversible account fingerprints and the duplicate check built on them.
//!
//! Two identifiers, deliberately kept apart:
//!
//! * The **fingerprint** is derived from the stable account id inside the credential and is used
//!   only to recognise "this is an account already added". It is irreversible, and it must never
//!   reach a log - logs identify accounts by their random internal id.
//! * **Post-switch verification** compares e-mail addresses, because `account/read` returns no
//!   stable id at all. That path lives in `switching`, not here.

use sha2::{Digest, Sha256};

/// Domain separator. Not a secret and not treated as one: it only keeps this digest from
/// colliding with a digest of the same input computed for some other purpose.
const DOMAIN: &[u8] = b"toglet.account-fingerprint.v1";

/// Derives the fingerprint from the stable account id found in the credential.
///
/// SHA-256 rather than a fast hash: the input is a UUID drawn from a small structured space, so
/// a non-cryptographic digest would be worth trying to invert. `sha2` is already in the
/// dependency graph through Tauri, so this costs nothing to add.
pub fn from_account_id(account_id: &str) -> String {
    digest(b"account-id", account_id.trim().as_bytes())
}

/// Fallback for credentials that carry no stable id.
///
/// Normalisation is **lowercase and trim, and nothing else**. That is the necessary scope, not
/// a shortcut: addresses reach Toglet from the app server, never from something a user typed,
/// so the provider-specific variants (dots and `+` aliases) cannot arise here. Applying Gmail's
/// dot rule to every domain would be actively wrong - on most providers `a.b@` and `ab@` are
/// different mailboxes.
pub fn from_email(email: &str) -> String {
    digest(b"email", normalize_email(email).as_bytes())
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn digest(kind: &[u8], value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(kind);
    hasher.update(value);
    // Hex rather than base64: it survives being pasted anywhere and has no `+` or `/` to
    // confuse a file name.
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What adding an account should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuplicateCheck {
    /// Not present; create a profile.
    New,
    /// Already present. The caller offers to refresh the existing account's credentials rather
    /// than creating a second profile.
    AlreadyPresent { existing_id: String },
}

/// Looks `fingerprint` up among `existing`, which pairs each known internal id with its
/// fingerprint.
pub fn check_duplicate<'a>(
    fingerprint: &str,
    existing: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> DuplicateCheck {
    for (id, known) in existing {
        if known == fingerprint {
            return DuplicateCheck::AlreadyPresent {
                existing_id: id.to_owned(),
            };
        }
    }
    DuplicateCheck::New
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_ID: &str = "8f14e45f-ceea-467a-9f3a-1c2d3e4f5a6b";

    #[test]
    fn a_fingerprint_is_stable_for_the_same_input() {
        assert_eq!(from_account_id(ACCOUNT_ID), from_account_id(ACCOUNT_ID));
    }

    #[test]
    fn a_fingerprint_does_not_contain_its_input() {
        let fingerprint = from_account_id(ACCOUNT_ID);

        assert!(!fingerprint.contains(ACCOUNT_ID));
        // No fragment of the id survives either.
        assert!(!fingerprint.contains("8f14e45f"));
        assert_eq!(fingerprint.len(), 64, "SHA-256 in hex");
    }

    #[test]
    fn an_email_fingerprint_does_not_contain_the_address() {
        let fingerprint = from_email("Leanne@Gmail.com");

        assert!(!fingerprint.contains("eanne"));
        assert!(!fingerprint.contains("gmail"));
    }

    #[test]
    fn case_and_padding_do_not_change_an_email_fingerprint() {
        assert_eq!(
            from_email("  Leanne@Gmail.COM  "),
            from_email("leanne@gmail.com")
        );
    }

    #[test]
    fn dots_and_aliases_are_left_alone_because_they_are_different_mailboxes() {
        // Deliberate: applying Gmail's dot rule everywhere would merge two real accounts on
        // most other providers. Addresses reach Toglet from the server, so variants of the
        // same account do not occur in practice.
        assert_ne!(from_email("a.b@example.com"), from_email("ab@example.com"));
        assert_ne!(
            from_email("user+work@example.com"),
            from_email("user@example.com")
        );
    }

    #[test]
    fn the_two_derivations_never_collide_for_the_same_text() {
        // Domain separation: an account id that happens to look like an address must not
        // produce the address fingerprint.
        assert_ne!(from_account_id("a@b.com"), from_email("a@b.com"));
    }

    #[test]
    fn different_inputs_give_different_fingerprints() {
        assert_ne!(from_account_id(ACCOUNT_ID), from_account_id("other-id"));
    }

    #[test]
    fn a_known_fingerprint_reports_the_existing_account_rather_than_adding_a_second() {
        let fingerprint = from_account_id(ACCOUNT_ID);
        let existing = vec![("acct-a", "some-other"), ("acct-b", fingerprint.as_str())];

        assert_eq!(
            check_duplicate(&fingerprint, existing),
            DuplicateCheck::AlreadyPresent {
                existing_id: "acct-b".to_owned()
            }
        );
    }

    #[test]
    fn an_unknown_fingerprint_is_new() {
        let existing = vec![("acct-a", "some-other")];

        assert_eq!(
            check_duplicate(&from_account_id(ACCOUNT_ID), existing),
            DuplicateCheck::New
        );
    }

    #[test]
    fn an_empty_account_list_is_new() {
        assert_eq!(
            check_duplicate("anything", std::iter::empty()),
            DuplicateCheck::New
        );
    }
}
