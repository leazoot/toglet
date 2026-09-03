//! What the command layer holds between calls.
//!
//! One place owns the application data directory, the metadata document, the credential store
//! and the two locks. Commands borrow them; nothing else in the crate does, which is what keeps
//! "who may write the default authentication" answerable by reading one file.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::codex_home::{create_private_dir, is_private};
use crate::credentials::{CredentialLock, SecretStore};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::storage::{MetadataDocument, MetadataStore};
use crate::switching::SwitchLock;

/// The directory name under the platform's application data location.
const APPLICATION_DIRECTORY: &str = "Toglet";

/// Where encrypted credentials live, inside the application directory.
const CREDENTIALS_DIRECTORY: &str = "credentials";

/// Everything the commands share.
pub struct AppState {
    data_directory: PathBuf,
    metadata: MetadataStore,
    secrets: Box<dyn SecretStore + Send + Sync>,
    /// The document, loaded once and kept in step with what is on disk.
    document: Mutex<MetadataDocument>,
    switch_lock: SwitchLock,
    credential_lock: CredentialLock,
}

impl AppState {
    /// Prepares the application data directory and loads what is in it.
    pub fn start() -> Result<Self> {
        let data_directory = default_data_directory()?;
        ensure_private_dir(&data_directory)?;
        let credentials = data_directory.join(CREDENTIALS_DIRECTORY);
        ensure_private_dir(&credentials)?;

        let metadata = MetadataStore::new(&data_directory);
        // A damaged document is rebuilt rather than fatal: the user would otherwise have no way
        // to reach the repair (`storage::MetadataStore::load`).
        let (document, _outcome) = metadata.load();

        Ok(Self {
            data_directory,
            metadata,
            secrets: platform_store(credentials)?,
            document: Mutex::new(document),
            switch_lock: SwitchLock::new(),
            credential_lock: CredentialLock::new(),
        })
    }

    pub fn secrets(&self) -> &dyn SecretStore {
        self.secrets.as_ref()
    }

    pub fn switch_lock(&self) -> &SwitchLock {
        &self.switch_lock
    }

    pub fn credential_lock(&self) -> &CredentialLock {
        &self.credential_lock
    }

    /// Where the switch journal lives.
    pub fn data_directory(&self) -> &Path {
        &self.data_directory
    }

    /// Runs `action` against the document and saves it if the action changed anything.
    ///
    /// Saving is inside the lock on purpose: two commands that both read, both modify and both
    /// save would otherwise lose one of the changes.
    pub fn with_document<T>(
        &self,
        action: impl FnOnce(&mut MetadataDocument) -> Result<(T, bool)>,
    ) -> Result<T> {
        let mut document = self
            .document
            .lock()
            // The guarded value is a plain document; a thread that panicked while holding this
            // has not corrupted it, and refusing every later command would be worse.
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let (value, changed) = action(&mut document)?;
        if changed {
            self.metadata.save(&document)?;
        }
        Ok(value)
    }

    /// Reads the document without the possibility of changing it.
    pub fn read_document<T>(&self, action: impl FnOnce(&MetadataDocument) -> T) -> T {
        let document = self
            .document
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        action(&document)
    }
}

/// The Codex home Codex itself would use.
pub fn codex_home() -> Result<PathBuf> {
    if let Some(explicit) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(explicit));
    }
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .map(|home| PathBuf::from(home).join(".codex"))
        .ok_or_else(|| {
            startup_error(
                ErrorCode::CodexHomeUnwritable,
                UserAction::InstallRuntime,
                "the Codex home could not be determined",
            )
        })
}

fn default_data_directory() -> Result<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    };

    base.map(|base| base.join(APPLICATION_DIRECTORY))
        .ok_or_else(|| {
            startup_error(
                ErrorCode::CodexHomeUnwritable,
                UserAction::None,
                "no application data directory could be determined",
            )
        })
}

/// Creates the directory, or accepts an existing one **only after checking it is private**.
///
/// `create_private_dir` refuses an existing name on purpose - a temporary directory that is
/// already there may be somebody else's. An application data directory is the opposite: it is
/// meant to survive between runs. Adopting it without checking its permissions would be exactly
/// the mistake that rule guards against, so the permissions are checked.
fn ensure_private_dir(path: &Path) -> Result<()> {
    match create_private_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => match is_private(path) {
            Ok(true) => Ok(()),
            Ok(false) => Err(startup_error(
                ErrorCode::CodexHomeUnwritable,
                UserAction::FixPermissions,
                "the application data directory is readable by others",
            )),
            Err(error) => Err(startup_error(
                ErrorCode::CodexHomeUnwritable,
                UserAction::FixPermissions,
                &error.to_string(),
            )),
        },
        Err(error) => Err(startup_error(
            ErrorCode::CodexHomeUnwritable,
            UserAction::FixPermissions,
            &error.to_string(),
        )),
    }
}

#[cfg(windows)]
fn platform_store(directory: PathBuf) -> Result<Box<dyn SecretStore + Send + Sync>> {
    Ok(Box::new(crate::credentials::WindowsSecretStore::new(
        directory,
    )))
}

#[cfg(target_os = "macos")]
fn platform_store(_directory: PathBuf) -> Result<Box<dyn SecretStore + Send + Sync>> {
    Ok(Box::new(crate::credentials::MacosSecretStore::new()))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_store(_directory: PathBuf) -> Result<Box<dyn SecretStore + Send + Sync>> {
    // Falling back to anything that stores plaintext is forbidden without exception, so an
    // unsupported platform fails to start instead.
    Err(startup_error(
        ErrorCode::CredentialStoreUnavailable,
        UserAction::None,
        "no credential store is implemented for this platform",
    ))
}

fn startup_error(code: ErrorCode, action: UserAction, detail: &str) -> TogletError {
    TogletError::new(code, Phase::Storage, false, action).with_detail(detail)
}
