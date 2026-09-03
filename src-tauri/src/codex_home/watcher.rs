//! Noticing that Codex changed the default authentication, without getting in its way.
//!
//! **Decided here: no watching crate; a stability check over file attributes.**
//! `notify` would deliver events sooner, but the requirement is not speed - a *debounce* is
//! needed regardless, so an event still has to be followed by "wait until it stops changing",
//! which is the whole of what this module does. Against that, a crate brings a
//! per-platform backend to reason about (on Windows an open directory handle), a dependency
//! tree, and one more place where "does it hold a lock on the user's credentials?" has to be
//! answered by reading somebody else's code. One file is being watched. The standard library
//! answers it in forty lines, and holds nothing between polls.
//!
//! The cost is honest and bounded: a change is noticed up to one poll interval late, and the
//! caller drives the polling. See [`AuthWatcher::poll`] for the case this cannot see at all.
//!
//! Like `quota::scheduler`, there is no thread here. State plus an observation in, a decision
//! out - so a write that arrives in the middle of another write can be tested without ever
//! sleeping.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many identical observations in a row mean the file has stopped changing.
///
/// Two rather than one: a single observation says nothing about whether a writer is still
/// working, and the gap between two of Codex's own writes would otherwise read as settled.
const STABLE_OBSERVATIONS: u32 = 2;

/// What the file looked like, cheaply. Never its contents - deciding whether to *read* the
/// credentials must not require reading the credentials first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    /// `None` on a platform or filesystem that does not report it; the length still varies.
    modified: Option<SystemTime>,
    len: u64,
}

/// What a poll concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthChange {
    /// Nothing has happened since the last settled observation.
    Unchanged,
    /// The file changed between observations, so it is still being written, or it could not be
    /// looked at this time. Reading now is exactly the half-written read this exists to avoid.
    Settling,
    /// The file has looked the same for long enough to be read.
    Settled,
    /// The file was there and is not any more - Codex signed out on its own.
    Vanished,
}

/// Watches the default `auth.json` by looking at it, never by holding it.
pub struct AuthWatcher {
    path: PathBuf,
    /// The most recent observation and how many polls in a row have matched it.
    last: Option<Stamp>,
    repeats: u32,
    /// The observation already reported as settled, so a quiet file stays quiet.
    reported: Option<Stamp>,
    /// Whether the file was present at the previous poll.
    existed: bool,
}

/// What a single look at the file found.
enum Look {
    Seen(Stamp),
    /// The file is not there.
    Gone,
    /// It exists, or may exist, but could not be looked at. Not the same as absent, and never
    /// reported as one.
    Unreadable,
}

impl AuthWatcher {
    /// Watches the `auth.json` of `home`.
    pub fn new(home: &Path) -> Self {
        Self {
            path: home.join("auth.json"),
            last: None,
            repeats: 0,
            reported: None,
            existed: false,
        }
    }

    /// Looks at the file once and says whether it is worth reading.
    ///
    /// The first poll over an existing file reports [`AuthChange::Settled`] after the second
    /// look, which is intended: at start-up Toglet has to reconcile with whatever is on disk
    /// anyway.
    ///
    /// **What this cannot see.** Two writes that leave the same length within the resolution of
    /// the filesystem's timestamp look identical, so the change is not noticed until the next
    /// one. It is not covered by tightening the check here - that would mean hashing the
    /// credentials on every poll - but by the synchronisation itself comparing bytes, and by
    /// the final synchronisation before exit, which runs whatever this last reported.
    pub fn poll(&mut self) -> AuthChange {
        match self.look() {
            Look::Unreadable => AuthChange::Settling,
            Look::Gone => {
                self.last = None;
                self.repeats = 0;
                self.reported = None;
                if std::mem::replace(&mut self.existed, false) {
                    AuthChange::Vanished
                } else {
                    AuthChange::Unchanged
                }
            }
            Look::Seen(stamp) => {
                self.existed = true;
                if self.last == Some(stamp) {
                    self.repeats = self.repeats.saturating_add(1);
                } else {
                    self.last = Some(stamp);
                    self.repeats = 1;
                }

                if self.reported == Some(stamp) {
                    return AuthChange::Unchanged;
                }
                if self.repeats < STABLE_OBSERVATIONS {
                    return AuthChange::Settling;
                }
                self.reported = Some(stamp);
                AuthChange::Settled
            }
        }
    }

    /// Asks the filesystem for attributes only.
    ///
    /// `metadata` does not give the watcher anything to keep: whatever handle the call needs is
    /// gone before it returns, so between polls Codex can open, rewrite or replace the file
    /// exactly as if Toglet were not running.
    fn look(&self) -> Look {
        match std::fs::metadata(&self.path) {
            Ok(metadata) => Look::Seen(Stamp {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Look::Gone,
            Err(_) => Look::Unreadable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::diagnostics::Phase;

    const SIGNED_IN: &[u8] = br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"rt-1"}}"#;

    fn home() -> IsolatedHome {
        IsolatedHome::create(Phase::Storage).expect("scratch home")
    }

    fn write(home: &IsolatedHome, contents: &[u8]) {
        std::fs::write(home.path().join("auth.json"), contents).expect("written");
    }

    #[test]
    fn a_file_that_is_never_there_is_never_reported_as_disappearing() {
        let home = home();
        let mut watcher = AuthWatcher::new(home.path());

        assert_eq!(watcher.poll(), AuthChange::Unchanged);
        assert_eq!(watcher.poll(), AuthChange::Unchanged);
    }

    #[test]
    fn an_existing_file_settles_and_then_stays_quiet() {
        let home = home();
        write(&home, SIGNED_IN);
        let mut watcher = AuthWatcher::new(home.path());

        assert_eq!(
            watcher.poll(),
            AuthChange::Settling,
            "one look proves nothing"
        );
        assert_eq!(watcher.poll(), AuthChange::Settled);
        assert_eq!(watcher.poll(), AuthChange::Unchanged);
        assert_eq!(watcher.poll(), AuthChange::Unchanged);
    }

    #[test]
    fn a_file_still_being_written_is_not_reported_as_ready_to_read() {
        // Each write changes the length, which is what a chunked writer looks like from the
        // outside.
        let home = home();
        write(&home, b"{");
        let mut watcher = AuthWatcher::new(home.path());
        assert_eq!(watcher.poll(), AuthChange::Settling);

        write(&home, br#"{"auth_mode":"#);
        assert_eq!(watcher.poll(), AuthChange::Settling);
        write(&home, SIGNED_IN);
        assert_eq!(
            watcher.poll(),
            AuthChange::Settling,
            "a file that changed since the last look must never be read"
        );

        assert_eq!(watcher.poll(), AuthChange::Settled);
    }

    #[test]
    fn a_new_sign_in_settles_again() {
        let home = home();
        write(&home, SIGNED_IN);
        let mut watcher = AuthWatcher::new(home.path());
        watcher.poll();
        assert_eq!(watcher.poll(), AuthChange::Settled);

        write(
            &home,
            br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"rt-rotated"}}"#,
        );

        assert_eq!(watcher.poll(), AuthChange::Settling);
        assert_eq!(watcher.poll(), AuthChange::Settled);
    }

    #[test]
    fn a_removed_file_is_reported_once_and_not_again() {
        let home = home();
        write(&home, SIGNED_IN);
        let mut watcher = AuthWatcher::new(home.path());
        watcher.poll();
        watcher.poll();

        std::fs::remove_file(home.path().join("auth.json")).expect("removed");

        assert_eq!(watcher.poll(), AuthChange::Vanished);
        assert_eq!(watcher.poll(), AuthChange::Unchanged);
    }

    #[test]
    fn a_file_that_comes_back_settles_again_rather_than_staying_quiet() {
        let home = home();
        write(&home, SIGNED_IN);
        let mut watcher = AuthWatcher::new(home.path());
        watcher.poll();
        watcher.poll();
        std::fs::remove_file(home.path().join("auth.json")).expect("removed");
        assert_eq!(watcher.poll(), AuthChange::Vanished);

        write(&home, SIGNED_IN);

        assert_eq!(watcher.poll(), AuthChange::Settling);
        assert_eq!(
            watcher.poll(),
            AuthChange::Settled,
            "signing back in must be noticed even if the file looks like it did before"
        );
    }

    /// The watcher must not hold the credentials open.
    ///
    /// Opening with a share mode of zero denies every other handle to the file. If the watcher
    /// kept one between polls - which is what a directory-watching backend would do - this open
    /// would fail with a sharing violation.
    #[cfg(windows)]
    #[test]
    fn the_watcher_holds_no_handle_between_polls() {
        use std::os::windows::fs::OpenOptionsExt;

        let home = home();
        write(&home, SIGNED_IN);
        let mut watcher = AuthWatcher::new(home.path());
        watcher.poll();
        watcher.poll();

        let exclusive = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(home.path().join("auth.json"));

        assert!(
            exclusive.is_ok(),
            "another process must be able to take the file exclusively: {exclusive:?}"
        );
    }

    /// The other half of the same requirement: Toglet must keep working while Codex holds the
    /// file, rather than concluding it has disappeared.
    #[cfg(windows)]
    #[test]
    fn a_file_another_process_holds_exclusively_is_not_reported_as_gone() {
        use std::os::windows::fs::OpenOptionsExt;

        let home = home();
        write(&home, SIGNED_IN);
        let mut watcher = AuthWatcher::new(home.path());
        watcher.poll();
        watcher.poll();

        let _held = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(home.path().join("auth.json"))
            .expect("the writer takes the file");

        assert_ne!(
            watcher.poll(),
            AuthChange::Vanished,
            "a file being written is not a file that was deleted"
        );
    }
}
