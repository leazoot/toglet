//! Remote-control settings commands. The shared secret is accepted but never returned: there is
//! no plaintext export. The bridge address is returned for editing; it is a capability URL that
//! can flood the command queue but cannot forge a signed command.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::state::AppState;
use super::views::ErrorView;
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::remote::store::{
    Bridge, LastCommandRecord, MIN_SECRET_LEN, RemoteConfig, RemoteStore, forget_bridge,
    store_bridge,
};
use crate::storage::LoadOutcome;

const PHASE: Phase = Phase::Remote;

/// The settings as the page shows them. Nothing here can reach a bridge.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteView {
    pub enabled: bool,
    /// Whether an address and a secret are stored.
    pub paired: bool,
    /// Host only, for display.
    pub bridge_host: String,
    /// Full address for editing; `None` when nothing is stored. The secret has no counterpart.
    pub bridge_endpoint: Option<String>,
    pub last_command: Option<LastCommandView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCommandView {
    /// Unix seconds.
    pub at: i64,
    pub action: String,
    /// Stable code for the interface to translate, never free text.
    pub result: String,
}

/// What the page sends when the user saves.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDraft {
    pub enabled: bool,
    /// `None` keeps the stored address.
    pub endpoint: Option<String>,
    /// `None` keeps the stored secret; the page never pre-fills it.
    pub secret: Option<String>,
}

pub struct Remote {
    store: RemoteStore,
    config: Mutex<RemoteConfig>,
}

impl Remote {
    /// Loads the settings. An unusable file is replaced by defaults, which have the feature off.
    pub fn load(data_directory: &std::path::Path) -> Self {
        let store = RemoteStore::new(data_directory);
        let (config, outcome) = store.load();
        if let LoadOutcome::Rebuilt { .. } = outcome {
            log(&LogRecord::new(Level::Warn, "remote_settings_rebuilt_at_start").with_phase(PHASE));
        }
        Self {
            store,
            config: Mutex::new(config),
        }
    }

    fn config(&self) -> std::sync::MutexGuard<'_, RemoteConfig> {
        self.config
            // Poisoning is harmless: the guarded value is a plain settings record.
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn snapshot(&self) -> RemoteConfig {
        self.config().clone()
    }

    /// Writes back what the poller changed: counter, nonce memory, last outcome.
    pub fn record(&self, updated: &RemoteConfig) -> Result<()> {
        let mut config = self.config();
        *config = updated.clone();
        self.store.save(&config)
    }

    fn view(&self, endpoint: Option<String>) -> RemoteView {
        let config = self.config();
        RemoteView {
            enabled: config.enabled,
            paired: endpoint.is_some(),
            bridge_host: config.bridge_host.clone(),
            bridge_endpoint: endpoint,
            last_command: config.last_command.as_ref().map(view_of),
        }
    }
}

fn view_of(record: &LastCommandRecord) -> LastCommandView {
    LastCommandView {
        at: record.at,
        action: record.action.clone(),
        result: record.result.clone(),
    }
}

/// The stored address, or `None` when unpaired. The secret is dropped immediately.
fn stored_endpoint(state: &AppState) -> Option<String> {
    crate::remote::store::load_bridge(state.secrets())
        .ok()
        .map(|bridge| bridge.endpoint)
}

#[tauri::command]
pub fn read_remote(state: State<'_, AppState>, remote: State<'_, Remote>) -> RemoteView {
    remote.view(stored_endpoint(&state))
}

/// Saves the settings and any newly typed bridge details. Turning the feature off deletes the
/// stored details, so no usable secret is left behind.
#[tauri::command]
pub fn save_remote(
    state: State<'_, AppState>,
    remote: State<'_, Remote>,
    draft: RemoteDraft,
) -> std::result::Result<RemoteView, ErrorView> {
    reported("remote_save_failed", save(&state, &remote, draft))
}

fn save(state: &AppState, remote: &Remote, draft: RemoteDraft) -> Result<RemoteView> {
    let secrets = state.secrets();

    if !draft.enabled {
        forget_bridge(secrets)?;
        let mut config = remote.config();
        config.disable();
        remote.store.save(&config)?;
        drop(config);
        return Ok(remote.view(None));
    }

    // An omitted field keeps the stored half, so one half of an existing pairing can be edited.
    // A first pairing still needs both.
    let held = crate::remote::store::load_bridge(secrets).ok();
    let bridge = match (
        draft
            .endpoint
            .or_else(|| held.as_ref().map(|b| b.endpoint.clone())),
        draft
            .secret
            .or_else(|| held.as_ref().map(|b| b.secret.clone())),
    ) {
        (Some(endpoint), Some(secret)) => Bridge { endpoint, secret },
        _ => {
            return Err(
                TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
                    .with_detail("an address and a secret are both needed the first time"),
            );
        }
    };
    bridge.validate()?;
    let host = bridge.host();
    store_bridge(secrets, &bridge)?;
    let endpoint = bridge.endpoint;

    let mut config = remote.config();
    config.enabled = true;
    config.bridge_host = host;
    remote.store.save(&config)?;
    drop(config);
    Ok(remote.view(Some(endpoint)))
}

/// Forgets the pairing entirely: the stored details, the host, and the replay state.
#[tauri::command]
pub fn forget_remote(
    state: State<'_, AppState>,
    remote: State<'_, Remote>,
) -> std::result::Result<RemoteView, ErrorView> {
    reported("remote_forget_failed", forget(&state, &remote))
}

fn forget(state: &AppState, remote: &Remote) -> Result<RemoteView> {
    forget_bridge(state.secrets())?;
    let mut config = remote.config();
    config.disable();
    remote.store.save(&config)?;
    drop(config);
    Ok(remote.view(None))
}

fn reported<T>(event: &'static str, outcome: Result<T>) -> std::result::Result<T, ErrorView> {
    outcome.map_err(|error| {
        log(&LogRecord::from_error(event, &error));
        ErrorView::from(error)
    })
}

/// Minimum secret length, so the page can validate before saving.
#[tauri::command]
pub fn remote_secret_minimum() -> usize {
    MIN_SECRET_LEN
}

#[cfg(test)]
mod tests {
    /// No command here may return the stored secret; the likely leak is a new view field.
    #[test]
    fn no_command_here_returns_anything_that_could_reach_a_bridge() {
        let source = include_str!("remote.rs");
        // Comments are stripped so prose naming a function does not count as a call.
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        // Two reads: `stored_endpoint` (address only) and `save` (the half not retyped).
        assert_eq!(implementation.matches("load_bridge").count(), 2);

        // Views may carry the host and the address, never the secret.
        let views = implementation
            .split("pub struct RemoteDraft")
            .next()
            .expect("the views come first");
        assert!(
            !views.contains("pub secret"),
            "a view must not carry the secret"
        );
        assert!(
            views.contains("pub bridge_endpoint"),
            "the address is meant to come back, so the page can offer it for editing"
        );
    }
}
