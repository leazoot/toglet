//! Remote settings: non-sensitive state in `remote.json`, bridge address and shared secret in
//! the credential store. No command may return the secret: there is no plaintext export.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::envelope::{Action, LastCommand, Outcome};
use crate::codex_home::atomic_write;
use crate::credentials::{CredentialRef, Secret, SecretStore};
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::storage::{LoadOutcome, LoadProblem};

const PHASE: Phase = Phase::Remote;

const REMOTE_FILE: &str = "remote.json";

pub const REMOTE_SCHEMA_VERSION: u32 = 1;

/// Domain separator for the binding fingerprint; not a secret.
const BINDING_DOMAIN: &[u8] = b"toglet.remote-binding.v1";

/// The settings file; holds no credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConfig {
    pub schema_version: u32,
    /// Off by default; turning it off deletes the credential entry.
    pub enabled: bool,
    /// Host only; the full address (whose path acts as a token) lives in the credential store.
    pub bridge_host: String,
    pub device_id: String,
    pub session_id: String,
    /// Irreversible digest of the bound session id, used to detect a rebind; `threadId` itself
    /// may not be stored here.
    pub binding_fingerprint: Option<String>,
    pub cursor: u64,
    pub recent_nonces: Vec<String>,
    pub last_command: Option<LastCommandRecord>,
}

/// The most recent command's outcome, echoed in the next receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCommandRecord {
    pub counter: u64,
    /// Stable strings rather than enums, so files survive builds with new variants.
    pub action: String,
    pub result: String,
    /// Unix seconds.
    pub at: i64,
}

impl LastCommandRecord {
    pub fn new(counter: u64, action: Action, result: Outcome, at: i64) -> Self {
        Self {
            counter,
            action: action.as_str().to_owned(),
            result: result.as_str().to_owned(),
            at,
        }
    }

    /// The receipt form, or `None` if this build cannot read what was written.
    pub fn to_envelope(&self) -> Option<LastCommand> {
        Some(LastCommand {
            counter: self.counter,
            action: Action::parse_str(&self.action)?,
            result: Outcome::parse(&self.result)?,
        })
    }
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            schema_version: REMOTE_SCHEMA_VERSION,
            enabled: false,
            bridge_host: String::new(),
            device_id: fresh_id(),
            session_id: fresh_id(),
            binding_fingerprint: None,
            cursor: 0,
            recent_nonces: Vec::new(),
            last_command: None,
        }
    }
}

impl RemoteConfig {
    /// On a different binding, mints a new `sessionId` and resets the counter so commands signed
    /// for the previous task cannot land on the new one. Returns whether anything changed.
    pub fn rebind(&mut self, fingerprint: Option<&str>) -> bool {
        if self.binding_fingerprint.as_deref() == fingerprint {
            return false;
        }
        self.binding_fingerprint = fingerprint.map(str::to_owned);
        self.session_id = fresh_id();
        self.cursor = 0;
        self.recent_nonces.clear();
        self.last_command = None;
        true
    }

    /// Clears the file-side state; the caller deletes the credential entry.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.bridge_host.clear();
        self.cursor = 0;
        self.recent_nonces.clear();
        self.last_command = None;
    }
}

/// An irreversible derivation of a bound session's id; the id itself may not be persisted here.
pub fn binding_fingerprint(thread_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(BINDING_DOMAIN);
    hasher.update(thread_id.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A fresh opaque identifier: unique, not secret, since forging a command needs the shared
/// secret. Hence clock, counter and a randomly seeded hasher rather than a CSPRNG.
fn fresh_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let counter = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(counter);

    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(now);
    hasher.write_u64(counter);
    let high = hasher.finish();

    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(high);
    hasher.write_u64(now.rotate_left(17));
    let low = hasher.finish();

    format!("{high:016x}{low:016x}")
}

pub struct RemoteStore {
    path: PathBuf,
}

impl RemoteStore {
    /// `directory` must be the existing, private application data directory.
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(REMOTE_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the settings; an unusable file is rebuilt with the feature off.
    pub fn load(&self) -> (RemoteConfig, LoadOutcome) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return (RemoteConfig::default(), LoadOutcome::Created);
        };

        let parsed = read_schema_version(&text)
            .ok_or(LoadProblem::Unreadable)
            .and_then(|version| {
                if version > REMOTE_SCHEMA_VERSION {
                    Err(LoadProblem::FromTheFuture { found: version })
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                serde_json::from_str::<RemoteConfig>(&text).map_err(|_| LoadProblem::Unreadable)
            });

        match parsed {
            Ok(config) => (config, LoadOutcome::Loaded),
            Err(problem) => {
                log(&LogRecord::new(Level::Error, "remote_settings_rebuilt")
                    .with_phase(PHASE)
                    .with_code(ErrorCode::Internal)
                    .with_detail(match problem {
                        LoadProblem::Unreadable => "the remote settings could not be parsed",
                        LoadProblem::FromTheFuture { .. } => {
                            "the remote settings were written by a newer version"
                        }
                    }));
                (RemoteConfig::default(), LoadOutcome::Rebuilt { problem })
            }
        }
    }

    pub fn save(&self, config: &RemoteConfig) -> Result<()> {
        let json = serde_json::to_vec_pretty(config).map_err(|error| {
            TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
                .with_detail(&error.to_string())
        })?;

        atomic_write(&self.path, &json).map_err(|error| {
            TogletError::new(
                ErrorCode::CodexHomeUnwritable,
                PHASE,
                true,
                UserAction::Retry,
            )
            .with_detail(&error.to_string())
        })
    }
}

/// The bridge address and shared secret, stored only in the credential store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bridge {
    pub endpoint: String,
    pub secret: String,
}

/// Minimum secret length: hard to guess, still typeable by hand on both ends.
pub const MIN_SECRET_LEN: usize = 16;
const MAX_SECRET_LEN: usize = 256;

impl Bridge {
    pub fn validate(&self) -> Result<()> {
        if !crate::net::is_safe_endpoint(&self.endpoint) {
            return Err(rejected("the bridge address"));
        }
        let secret = self.secret.as_bytes();
        let usable = secret.len() >= MIN_SECRET_LEN
            && secret.len() <= MAX_SECRET_LEN
            && self.secret.is_ascii()
            && !secret.iter().any(|byte| *byte <= b' ' || *byte == 0x7f);
        if usable {
            Ok(())
        } else {
            Err(rejected("the shared secret"))
        }
    }

    pub fn host(&self) -> String {
        crate::net::host_of(&self.endpoint)
            .unwrap_or_default()
            .to_owned()
    }
}

/// Credential key, prefixed to avoid colliding with account (`cred-…`) and channel (`notify-…`) keys.
fn reference() -> Result<CredentialRef> {
    CredentialRef::new("remote-bridge")
}

pub fn store_bridge(secrets: &dyn SecretStore, bridge: &Bridge) -> Result<()> {
    bridge.validate()?;
    let json = serde_json::to_vec(bridge).map_err(|error| {
        TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
            .with_detail(&error.to_string())
    })?;
    secrets.store(&reference()?, &Secret::new(json))
}

/// Reads the bridge details back; the secret must never be returned by any command.
pub fn load_bridge(secrets: &dyn SecretStore) -> Result<Bridge> {
    let secret = secrets.load(&reference()?)?;
    serde_json::from_slice::<Bridge>(secret.expose()).map_err(|_| {
        TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
            .with_detail("the stored bridge details could not be read back")
    })
}

/// Removes the bridge details; succeeds if there are none.
pub fn forget_bridge(secrets: &dyn SecretStore) -> Result<()> {
    secrets.delete(&reference()?)
}

/// The rejected value is never quoted: errors reach the interface and the log.
fn rejected(what: &str) -> TogletError {
    TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
        .with_detail(&format!("{what} is not usable"))
}

fn read_schema_version(text: &str) -> Option<u32> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct VersionOnly {
        schema_version: u32,
    }
    serde_json::from_str::<VersionOnly>(text)
        .ok()
        .map(|parsed| parsed.schema_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;
    use crate::credentials::MemorySecretStore;

    fn bridge() -> Bridge {
        Bridge {
            endpoint: "https://bridge.example.com/toglet".to_owned(),
            secret: "a-secret-long-enough".to_owned(),
        }
    }

    #[test]
    fn a_fresh_installation_has_the_feature_off_and_no_host() {
        let config = RemoteConfig::default();
        assert!(!config.enabled);
        assert!(config.bridge_host.is_empty());
        assert_eq!(config.cursor, 0);
        assert!(config.last_command.is_none());
    }

    #[test]
    fn two_identifiers_minted_in_a_row_differ() {
        let first = RemoteConfig::default();
        let second = RemoteConfig::default();
        assert_ne!(first.device_id, second.device_id);
        assert_ne!(first.session_id, first.device_id);
        assert_eq!(first.session_id.len(), 32);
        assert!(first.session_id.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn binding_to_a_different_session_starts_a_new_sequence() {
        let mut config = RemoteConfig {
            cursor: 17,
            recent_nonces: vec!["abc".to_owned()],
            ..RemoteConfig::default()
        };
        let first_session = config.session_id.clone();

        assert!(config.rebind(Some(&binding_fingerprint("thread-1"))));
        assert_ne!(config.session_id, first_session);
        assert_eq!(config.cursor, 0);
        assert!(config.recent_nonces.is_empty());
    }

    #[test]
    fn binding_to_the_same_session_again_changes_nothing() {
        let mut config = RemoteConfig::default();
        let fingerprint = binding_fingerprint("thread-1");
        config.rebind(Some(&fingerprint));
        let settled = config.clone();

        assert!(!config.rebind(Some(&fingerprint)));
        assert_eq!(config, settled);
    }

    #[test]
    fn a_fingerprint_is_not_the_session_it_was_made_from() {
        let fingerprint = binding_fingerprint("0199-abcd-thread");
        assert!(!fingerprint.contains("0199"));
        assert!(!fingerprint.contains("thread"));
        assert_eq!(fingerprint.len(), 32);
        assert_eq!(fingerprint, binding_fingerprint("0199-abcd-thread"));
        assert_ne!(fingerprint, binding_fingerprint("0199-abcd-threae"));
    }

    #[test]
    fn turning_the_feature_off_forgets_the_host_and_the_replay_state() {
        let mut config = RemoteConfig {
            enabled: true,
            bridge_host: "bridge.example.com".to_owned(),
            cursor: 9,
            recent_nonces: vec!["abc".to_owned()],
            ..RemoteConfig::default()
        };

        config.disable();

        assert!(!config.enabled);
        assert!(config.bridge_host.is_empty());
        assert_eq!(config.cursor, 0);
        assert!(config.recent_nonces.is_empty());
    }

    #[test]
    fn a_bridge_is_refused_unless_both_halves_are_usable() {
        assert!(bridge().validate().is_ok());

        let cases = [
            ("http://bridge.example.com/x", "a-secret-long-enough"),
            ("https://bridge.example.com/x", "short"),
            ("https://bridge.example.com/x", "has a space in it....."),
            (
                "https://bridge.example.com/x\r\nHost: x",
                "a-secret-long-enough",
            ),
        ];
        for (endpoint, secret) in cases {
            let candidate = Bridge {
                endpoint: endpoint.to_owned(),
                secret: secret.to_owned(),
            };
            assert!(
                candidate.validate().is_err(),
                "should have been refused: {endpoint}"
            );
        }
    }

    #[test]
    fn refusing_a_bridge_never_quotes_it() {
        let candidate = Bridge {
            endpoint: "http://secret-host.example.com/x".to_owned(),
            secret: "correct-horse-battery".to_owned(),
        };
        let error = candidate.validate().expect_err("refused");
        let rendered = format!("{error:?}");
        assert!(!rendered.contains("secret-host"));
        assert!(!rendered.contains("correct-horse"));
    }

    #[test]
    fn the_file_on_disk_holds_no_address_and_no_secret() {
        let home = IsolatedHome::create(PHASE).expect("a temporary directory");
        let store = RemoteStore::new(home.path());
        let secrets = MemorySecretStore::new();
        store_bridge(&secrets, &bridge()).expect("stored");

        let config = RemoteConfig {
            enabled: true,
            bridge_host: bridge().host(),
            ..RemoteConfig::default()
        };
        store.save(&config).expect("saved");

        let written = std::fs::read_to_string(store.path()).expect("written");
        assert!(written.contains("bridge.example.com"), "the host is shown");
        for forbidden in ["https://", "/toglet", "a-secret-long-enough"] {
            assert!(
                !written.contains(forbidden),
                "remote.json must not contain {forbidden}: {written}"
            );
        }
    }

    #[test]
    fn the_bridge_details_survive_a_round_trip_and_can_be_forgotten() {
        let secrets = MemorySecretStore::new();
        store_bridge(&secrets, &bridge()).expect("stored");
        assert_eq!(load_bridge(&secrets).expect("loaded"), bridge());

        forget_bridge(&secrets).expect("forgotten");
        assert!(load_bridge(&secrets).is_err());
        forget_bridge(&secrets).expect("forgetting twice is not an error");
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let home = IsolatedHome::create(PHASE).expect("a temporary directory");
        let store = RemoteStore::new(home.path());

        let config = RemoteConfig {
            enabled: true,
            bridge_host: "bridge.example.com".to_owned(),
            cursor: 7,
            recent_nonces: vec!["a".to_owned(), "b".to_owned()],
            last_command: Some(LastCommandRecord::new(
                7,
                Action::Resume,
                Outcome::Applied,
                1_757_664_000,
            )),
            ..RemoteConfig::default()
        };
        store.save(&config).expect("saved");

        let (read, outcome) = store.load();
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(read, config);
        assert_eq!(
            read.last_command.and_then(|last| last.to_envelope()),
            Some(LastCommand {
                counter: 7,
                action: Action::Resume,
                result: Outcome::Applied,
            })
        );
    }

    #[test]
    fn a_damaged_file_is_rebuilt_with_the_feature_off() {
        let home = IsolatedHome::create(PHASE).expect("a temporary directory");
        let store = RemoteStore::new(home.path());
        std::fs::write(store.path(), b"{ not json").expect("written");

        let (config, outcome) = store.load();
        assert!(!config.enabled);
        assert!(matches!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::Unreadable
            }
        ));
    }

    #[test]
    fn a_file_from_a_newer_build_is_refused_rather_than_guessed_at() {
        let home = IsolatedHome::create(PHASE).expect("a temporary directory");
        let store = RemoteStore::new(home.path());
        std::fs::write(
            store.path(),
            format!(r#"{{"schemaVersion":{}}}"#, REMOTE_SCHEMA_VERSION + 1),
        )
        .expect("written");

        let (config, outcome) = store.load();
        assert!(!config.enabled);
        assert!(matches!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::FromTheFuture { .. }
            }
        ));
    }

    #[test]
    fn a_last_command_this_build_cannot_read_is_not_invented() {
        let record = LastCommandRecord {
            counter: 1,
            action: "teleport".to_owned(),
            result: "applied".to_owned(),
            at: 0,
        };
        assert!(record.to_envelope().is_none());
    }
}
