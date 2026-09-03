//! Creating directories and files that only the current OS user can read.
//!
//! Permissions are applied at creation time, never after the content is written: a file that
//! exists for even an instant with a default ACL is a file another process may already have
//! opened. Both platform implementations below therefore hand the permissions to the create
//! call itself rather than fixing them up afterwards.
//!
//! Deletion is best effort; permissions are the first line of defence.
//!
//! A fuller `FilePermissions` abstraction (application data directory, `auth.json`, backups)
//! must extend this module - a second implementation of "make it private" would be exactly the
//! kind of duplication this module exists to prevent.

use std::io;
use std::path::Path;

#[cfg(unix)]
#[path = "permissions_unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "permissions_windows.rs"]
mod imp;

/// Creates `path` as a directory that only the current user may enter or read.
///
/// Fails with [`io::ErrorKind::AlreadyExists`] when the name is taken. Callers depend on that:
/// a temporary directory that already exists may have been created by somebody else, so the
/// correct response is to pick another name, never to adopt it.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    imp::create_private_dir(path)
}

/// Creates `path` with the permissions already applied and writes `contents` to it.
///
/// Fails if the file exists - this never overwrites, and never widens an existing file's
/// permissions.
pub(crate) fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;

    let mut file = create_private_file(path)?;
    file.write_all(contents)
}

/// Creates `path` with its permissions already applied and hands back the open handle.
///
/// Used where the caller has to do more than write - `atomic_write` needs to `fsync` before the
/// handle closes.
pub(crate) fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    imp::create_private_file(path)
}

/// Opens `path` for appending, creating it privately if it is not there yet.
///
/// Exists here rather than in `diagnostics` because `diagnostics` is a leaf module and may not
/// depend on a business module. Keeping the one implementation of "make it private" in this
/// module is what stops a second one appearing next to the logger.
pub fn open_private_append(path: &Path) -> io::Result<std::fs::File> {
    if !path.exists() {
        // Permissions are applied by the creation itself, so the file is never briefly
        // readable by anyone else - not even while it is empty.
        drop(imp::create_private_file(path)?);
    }
    std::fs::OpenOptions::new().append(true).open(path)
}

/// Whether `path` is readable by the current OS user and nobody else.
///
/// On Windows this means a protected DACL holding exactly one access-allowed ACE for the token
/// user. That single-ACE check is what rules out `Everyone`, `Users` and `Authenticated Users`:
/// resolving those well-known SIDs separately would add unsafe code without adding assurance,
/// because an ACE for any of them would already have failed the count.
///
/// On POSIX it means no group or other bits are set.
pub fn is_private(path: &Path) -> io::Result<bool> {
    imp::is_private(path)
}

/// Panics unless `path` is readable by the current user only.
#[cfg(test)]
pub(crate) fn assert_private(path: &Path) {
    assert!(
        is_private(path).expect("the path's permissions are readable"),
        "the path is reachable by someone other than the current user"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::diagnostics::Phase;

    /// Every one of these is a real failure, not an injected one: a permission call that fails
    /// returns an error the caller has to handle, never a warning it can ignore.
    #[test]
    fn a_directory_that_cannot_be_created_reports_an_error() {
        let scratch = IsolatedHome::create(Phase::Storage).expect("scratch directory");

        let error = create_private_dir(&scratch.path().join("absent").join("child"))
            .expect_err("creating under a missing parent must fail");

        assert_ne!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn creating_a_directory_that_already_exists_reports_already_exists() {
        let scratch = IsolatedHome::create(Phase::Storage).expect("scratch directory");

        let error = create_private_dir(scratch.path())
            .expect_err("an existing directory must not be adopted");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn writing_over_an_existing_file_is_refused_rather_than_widening_it() {
        let scratch = IsolatedHome::create(Phase::Storage).expect("scratch directory");
        let path = scratch.path().join("once");
        write_private_file(&path, b"first").expect("the first write succeeds");

        let error = write_private_file(&path, b"second").expect_err("the second write is refused");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(&path).expect("still readable"),
            b"first",
            "the refused write must not have touched the content"
        );
    }

    #[test]
    fn checking_a_path_that_is_not_there_is_an_error_not_a_false() {
        let scratch = IsolatedHome::create(Phase::Storage).expect("scratch directory");

        assert!(
            is_private(&scratch.path().join("absent")).is_err(),
            "an unreadable path must not be reported as private"
        );
    }
}
