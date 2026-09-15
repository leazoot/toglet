//! State the command layer holds between calls: data directory, metadata document, credential
//! store and locks. Only commands borrow them, so who may write the default authentication is
//! answerable from this one file.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::codex_home::{create_private_dir, is_private, open_private_append};
use crate::credentials::{CredentialLock, SecretStore};
use crate::diagnostics::{
    ErrorCode, LOG_FILE_NAME, Level, LogRecord, Logger, Phase, Result, TogletError, UserAction,
    install, log,
};
use crate::storage::{MetadataDocument, MetadataStore};
use crate::switching::SwitchLock;

/// The directory name under the platform's application data location.
const APPLICATION_DIRECTORY: &str = "Toglet";

/// Credential store directory, inside the application directory.
const CREDENTIALS_DIRECTORY: &str = "credentials";

/// Everything the commands share. Clones are cheap handles to the same core; the autorun driver
/// thread holds one.
#[derive(Clone)]
pub struct AppState(Arc<Core>);

struct Core {
    data_directory: PathBuf,
    metadata: MetadataStore,
    secrets: Box<dyn SecretStore + Send + Sync>,
    /// Loaded once and kept in step with what is on disk.
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
        // A damaged document is rebuilt rather than fatal, or the user could not reach the repair.
        let (document, _outcome) = metadata.load();

        Ok(Self(Arc::new(Core {
            data_directory,
            metadata,
            secrets: platform_store(credentials)?,
            document: Mutex::new(document),
            switch_lock: SwitchLock::new(),
            credential_lock: CredentialLock::new(),
        })))
    }

    pub fn secrets(&self) -> &dyn SecretStore {
        self.0.secrets.as_ref()
    }

    pub fn switch_lock(&self) -> &SwitchLock {
        &self.0.switch_lock
    }

    pub fn credential_lock(&self) -> &CredentialLock {
        &self.0.credential_lock
    }

    /// Where the switch journal lives.
    pub fn data_directory(&self) -> &Path {
        &self.0.data_directory
    }

    /// Runs `action` on the document and saves if it changed anything. Saving inside the lock
    /// prevents lost updates between concurrent commands.
    pub fn with_document<T>(
        &self,
        action: impl FnOnce(&mut MetadataDocument) -> Result<(T, bool)>,
    ) -> Result<T> {
        let mut document = self
            .0
            .document
            .lock()
            // Poisoning is harmless: the guarded value is a plain document.
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let (value, changed) = action(&mut document)?;
        if changed {
            self.0.metadata.save(&document)?;
        }
        Ok(value)
    }

    pub fn read_document<T>(&self, action: impl FnOnce(&MetadataDocument) -> T) -> T {
        let document = self
            .0
            .document
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        action(&document)
    }
}

/// Starts writing `toglet.log` in the data directory and drains records logged before it existed.
/// Records are redacted when built. If the file cannot be opened, that is logged in memory and
/// start-up continues. Returns whether the file is being written.
pub fn install_file_log(directory: &Path) -> bool {
    let file = match open_private_append(&directory.join(LOG_FILE_NAME)) {
        Ok(file) => file,
        Err(error) => {
            log(&LogRecord::new(Level::Warn, "log_file_unavailable")
                .with_phase(Phase::Storage)
                .with_detail(&error.to_string()));
            return false;
        }
    };
    // A second install is a programming error, not a runtime condition: the first sink stays.
    let installed = install(Logger::new(file)).is_ok();
    log(&LogRecord::new(Level::Info, "started")
        .with_phase(Phase::Detect)
        .with_detail(env!("CARGO_PKG_VERSION")));
    installed
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

/// Creates the directory, or accepts an existing one only after checking it is private
/// (`create_private_dir` alone refuses existing directories).
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
fn platform_store(directory: PathBuf) -> Result<Box<dyn SecretStore + Send + Sync>> {
    // Not the login Keychain: without an Apple signing identity it prompts for the login password
    // on every rebuilt binary. Private files match how Codex protects `auth.json`.
    Ok(Box::new(crate::credentials::FileSecretStore::new(
        directory,
    )))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_store(_directory: PathBuf) -> Result<Box<dyn SecretStore + Send + Sync>> {
    // No plaintext fallback: an unsupported platform fails to start instead.
    Err(startup_error(
        ErrorCode::CredentialStoreUnavailable,
        UserAction::None,
        "no credential store is implemented for this platform",
    ))
}

fn startup_error(code: ErrorCode, action: UserAction, detail: &str) -> TogletError {
    TogletError::new(code, Phase::Storage, false, action).with_detail(detail)
}
