//! Creating directories and files that only the current OS user can read.
//! Permissions are applied by the create call itself, never after content is written. Deletion is
//! best effort; permissions are the first line of defence.

use std::io;
use std::path::Path;

#[cfg(unix)]
#[path = "permissions_unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "permissions_windows.rs"]
mod imp;

/// Creates `path` as a directory only the current user may enter or read.
/// Fails with `AlreadyExists` if taken: an existing directory may belong to someone else, so
/// callers pick another name rather than adopt it.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    imp::create_private_dir(path)
}

/// Creates `path` with the permissions already applied and writes `contents`. Never overwrites.
pub(crate) fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;

    let mut file = create_private_file(path)?;
    file.write_all(contents)
}

/// Creates `path` with its permissions already applied and returns the open handle.
pub(crate) fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    imp::create_private_file(path)
}

/// Opens `path` for appending, creating it privately first. Lives here because `diagnostics` is
/// a leaf module.
pub fn open_private_append(path: &Path) -> io::Result<std::fs::File> {
    if !path.exists() {
        // Created with permissions applied, so it is never briefly readable, even while empty.
        drop(imp::create_private_file(path)?);
    }
    std::fs::OpenOptions::new().append(true).open(path)
}

/// Whether `path` is readable by the current OS user and nobody else.
/// Windows: a protected DACL with exactly one allow ACE for the token user, which also rules out
/// `Everyone` and `Users`. POSIX: no group or other bits.
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
