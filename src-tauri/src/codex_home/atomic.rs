//! The single atomic write used by every persistence path.
//! A temporary file in the target's own directory (rename is only atomic within one filesystem),
//! created private → content → `fsync` → rename → directory `fsync`.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::permissions;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Replaces `path`'s content atomically: afterwards it holds the complete old or new content.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    stage(path, contents)?.commit()
}

/// Writes the replacement without installing it, so a switch can fail between staging and
/// committing and still roll back. [`atomic_write`] is these two in a row.
pub fn stage(path: &Path, contents: &[u8]) -> io::Result<Staged> {
    Staged::write(path, contents)
}

/// A fully written temporary file waiting to replace its target. Removed if dropped uncommitted.
pub struct Staged {
    temporary: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl Staged {
    fn write(target: &Path, contents: &[u8]) -> io::Result<Self> {
        let directory = target.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the target has no parent directory",
            )
        })?;
        let file_name = target.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "the target has no file name")
        })?;

        let temporary = directory.join(format!(
            "{}.toglet-tmp-{:016x}",
            file_name.to_string_lossy(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let staged = Self {
            temporary,
            target: target.to_path_buf(),
            committed: false,
        };

        // Guard is armed before the first byte: every failure below removes the temporary file.
        let mut file = permissions::create_private_file(&staged.temporary)?;
        file.write_all(contents)?;
        // Without this the rename can be durable while the content is not.
        file.sync_all()?;
        drop(file);

        Ok(staged)
    }

    /// Replaces the target with the staged file.
    pub fn commit(mut self) -> io::Result<()> {
        // On Windows `rename` replaces an existing target, and Codex holds no exclusive lock on it.
        std::fs::rename(&self.temporary, &self.target)?;
        self.committed = true;
        sync_directory(&self.target);
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.committed {
            // Best effort: a leftover is user-only, so it is clutter, not a leak.
            drop(std::fs::remove_file(&self.temporary));
        }
    }
}

/// Flushes the directory entry so the rename survives a power loss.
/// POSIX only: Windows would need `FILE_FLAG_BACKUP_SEMANTICS`, and NTFS journals the rename.
#[cfg(unix)]
fn sync_directory(target: &Path) {
    if let Some(directory) = target.parent() {
        if let Ok(handle) = std::fs::File::open(directory) {
            drop(handle.sync_all());
        }
    }
}

#[cfg(not(unix))]
fn sync_directory(_target: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::diagnostics::Phase;

    fn scratch() -> IsolatedHome {
        IsolatedHome::create(Phase::Storage).expect("scratch directory")
    }

    #[test]
    fn writing_a_new_file_leaves_it_private() {
        let home = scratch();
        let path = home.path().join("metadata.json");

        atomic_write(&path, b"first").expect("the write succeeds");

        assert_eq!(std::fs::read(&path).expect("readable"), b"first");
        permissions::assert_private(&path);
    }

    #[test]
    fn replacing_a_file_keeps_it_private() {
        let home = scratch();
        let path = home.path().join("metadata.json");
        atomic_write(&path, b"first").expect("the first write succeeds");

        atomic_write(&path, b"second-and-longer").expect("the replace succeeds");

        assert_eq!(
            std::fs::read(&path).expect("readable"),
            b"second-and-longer"
        );
        // The rename carries the temporary file's permissions onto the target.
        permissions::assert_private(&path);
    }

    #[test]
    fn an_interruption_before_the_rename_leaves_the_original_untouched() {
        let home = scratch();
        let path = home.path().join("metadata.json");
        atomic_write(&path, b"original").expect("the first write succeeds");

        // Stands in for a crash between writing and renaming: the staged file is dropped
        // without being committed.
        let staged = Staged::write(&path, b"replacement").expect("staging succeeds");
        let temporary = staged.temporary.clone();
        drop(staged);

        assert_eq!(
            std::fs::read(&path).expect("readable"),
            b"original",
            "an interrupted write must not have touched the target"
        );
        assert!(!temporary.exists(), "the staged file was left behind");
    }

    #[test]
    fn the_temporary_file_is_a_sibling_of_the_target() {
        let home = scratch();
        let path = home.path().join("metadata.json");

        let staged = Staged::write(&path, b"x").expect("staging succeeds");

        assert_eq!(
            staged.temporary.parent(),
            path.parent(),
            "a temporary file on another filesystem would break the atomic rename"
        );
    }

    #[test]
    fn two_concurrent_writes_do_not_reuse_a_temporary_name() {
        let home = scratch();
        let first = Staged::write(&home.path().join("a.json"), b"a").expect("staging succeeds");
        let second = Staged::write(&home.path().join("a.json"), b"b").expect("staging succeeds");

        assert_ne!(first.temporary, second.temporary);
    }

    #[test]
    fn a_target_without_a_directory_is_rejected_rather_than_guessed() {
        let error = atomic_write(Path::new(""), b"x").expect_err("an empty path is rejected");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    /// On Windows a target another process holds open cannot be replaced; that must surface as an
    /// error, never as a silent no-op.
    #[cfg(windows)]
    #[test]
    fn a_target_another_process_holds_open_fails_loudly_and_changes_nothing() {
        use std::os::windows::fs::OpenOptionsExt;

        let home = scratch();
        let path = home.path().join("auth.json");
        atomic_write(&path, b"original").expect("the first write succeeds");

        let held = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(&path)
            .expect("the other process takes the file");

        let error = atomic_write(&path, b"replacement")
            .expect_err("replacing a file that is held open must not report success");

        // The rename fails with OS error 5 (`PermissionDenied`); the kind is asserted.
        assert_eq!(
            error.kind(),
            io::ErrorKind::PermissionDenied,
            "the caller needs a distinguishable failure: {error:?}"
        );
        assert!(
            error.raw_os_error().is_some(),
            "the OS code must survive so it can be reported: {error:?}"
        );
        assert!(
            leftover_temporaries(home.path()).is_empty(),
            "a failed replace must not leave a staged copy of the content behind"
        );

        drop(held);
        assert_eq!(
            std::fs::read(&path).expect("readable"),
            b"original",
            "a failed replace must leave the previous content intact"
        );
    }

    #[cfg(windows)]
    fn leftover_temporaries(directory: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(directory)
            .expect("readable")
            .filter_map(|entry| {
                let path = entry.expect("entry").path();
                path.file_name()?
                    .to_string_lossy()
                    .contains("toglet-tmp-")
                    .then_some(path)
            })
            .collect()
    }
}
