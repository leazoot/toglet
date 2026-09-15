//! Noticing that Codex changed the default authentication, without getting in its way.
//! Polls file attributes until they stop changing; no watching crate, no handle held between
//! polls, and no thread: the caller drives polling, so the logic is testable without sleeping.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Identical observations in a row that mean the file has stopped changing. Two, because one
/// says nothing about whether a writer is still working.
const STABLE_OBSERVATIONS: u32 = 2;

/// What the file looked like, never its contents: deciding whether to read the credentials must
/// not require reading them.
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
    /// Two writes leaving the same length within one timestamp tick look identical; that is
    /// covered by the synchronisation comparing bytes, not by hashing credentials here.
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

    /// Reads attributes only. No handle outlives the call, so Codex can rewrite or replace the file
    /// freely between polls.
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

    /// A share-mode-0 open fails if the watcher still holds a handle to the file.
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

    /// Toglet must keep working while Codex holds the file, not conclude it vanished.
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
