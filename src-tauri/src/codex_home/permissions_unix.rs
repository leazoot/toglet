//! POSIX implementation: `0700` directories and `0600` files.
//!
//! Not verified on a real macOS machine yet.

use std::fs::{DirBuilder, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

pub(super) fn create_private_dir(path: &Path) -> io::Result<()> {
    // mkdir(2) applies the mode as part of creating the directory, so there is no window in
    // which it is world-readable. umask can only clear further bits, never add them, so the
    // result is never more permissive than 0700.
    DirBuilder::new().mode(0o700).create(path)
}

pub(super) fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    // `create_new` plus `mode` means open(2) receives O_CREAT | O_EXCL and the mode together:
    // the file cannot exist beforehand and is never briefly group- or world-readable.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// No group or other access. That is the actual security property; the exact mode matters
/// less than the absence of those bits.
pub(super) fn is_private(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)?;
    Ok(metadata.permissions().mode() & 0o077 == 0)
}
