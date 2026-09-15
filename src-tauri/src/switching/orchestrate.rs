//! A whole switch in the only safe order: check, close the clients, replace, verify.
//! Shared by the manual switch and automatic continuation so neither can skip a step. It never
//! records `activeAccountId`; the returned `SwitchVerified` lets the caller do that.

use std::path::Path;

use super::preflight::{ClientVerdict, Preflight, SwitchLock, SwitchTarget};
use super::state::{StepObserver, SwitchProgress};
use super::swap::{Faults, RollbackReport, Switch};
use crate::accounts::external_change::ActiveAccount;
use crate::app_server::CodexBinary;
use crate::credentials::{CredentialLock, CredentialRef, SecretStore};
use crate::diagnostics::{Result, TogletError};
use crate::process::{self, ClientProbe, ClientRestart, RestartPlan, SHUTDOWN_TIMEOUT};
use crate::storage::SwitchVerified;

/// Everything a switch needs from the application.
pub struct SwitchContext<'a> {
    pub lock: &'a SwitchLock,
    pub credential_lock: &'a CredentialLock,
    pub store: &'a dyn SecretStore,
    pub probe: &'a dyn ClientProbe,
    pub restart: &'a dyn ClientRestart,
    pub binary: &'a CodexBinary,
    /// The user's real Codex home - the one the switch writes to.
    pub default_home: &'a Path,
    /// Where the journal lives: the application's own data directory.
    pub journal_directory: &'a Path,
    /// Process ids Toglet started itself, which must not block Toglet's own switch.
    pub own_processes: &'a [u32],
    pub faults: &'a dyn Faults,
    pub observer: &'a dyn StepObserver,
}

/// The account Codex is signed in as now, as Toglet has it on record.
#[derive(Debug, Clone, Copy)]
pub struct ActiveRecord<'a> {
    pub account_id: &'a str,
    pub credentials: &'a CredentialRef,
    /// Its `accountFingerprint`; not for logs or the frontend.
    pub fingerprint: &'a str,
}

/// How a switch that got past the pre-checks ended.
#[derive(Debug)]
pub enum SwitchReport {
    /// Replaced and verified; `plan` says which clients to reopen.
    Switched {
        verified: SwitchVerified,
        progress: SwitchProgress,
        verdict: ClientVerdict,
        plan: RestartPlan,
    },
    /// Stopped after the pre-checks; `rollback` says what happened to the previous authentication.
    Failed {
        error: TogletError,
        progress: SwitchProgress,
        verdict: ClientVerdict,
        rollback: RollbackReport,
    },
}

/// Runs a switch to `target`.
///
/// `Err` means a pre-check failed or a client would not close, before any backup, so nothing
/// needs rolling back. Later failures are reported as `SwitchReport::Failed`.
pub fn perform(
    context: &SwitchContext<'_>,
    active: Option<ActiveRecord<'_>>,
    target: SwitchTarget<'_>,
    operation_id: &str,
    started_at: &str,
) -> Result<SwitchReport> {
    let preflight = Preflight {
        lock: context.lock,
        credential_lock: context.credential_lock,
        store: context.store,
        probe: context.probe,
        binary: context.binary,
        default_home: context.default_home,
        own_processes: context.own_processes,
    };
    let passed = preflight
        .run(
            active.map(|record| record.account_id),
            active.map(|record| ActiveAccount {
                credentials: record.credentials,
                fingerprint: record.fingerprint,
            }),
            SwitchTarget {
                account_id: target.account_id,
                credentials: target.credentials,
            },
        )
        .map_err(|failure| failure.error)?;

    let verdict = passed.verdict;
    let plan = process::plan(&passed.clients);
    // A client that will not close stops the switch before the credentials are touched.
    process::close(context.restart, &plan, SHUTDOWN_TIMEOUT)?;

    let switch = Switch {
        binary: context.binary,
        default_home: context.default_home,
        journal_directory: context.journal_directory,
        faults: context.faults,
        observer: context.observer,
    };
    Ok(
        match switch.run(
            passed,
            active.map(|record| record.account_id),
            target.account_id,
            operation_id,
            started_at,
        ) {
            Ok(succeeded) => SwitchReport::Switched {
                verified: succeeded.verified,
                progress: succeeded.progress,
                verdict,
                plan,
            },
            Err(failed) => SwitchReport::Failed {
                error: failed.error,
                progress: failed.progress,
                verdict,
                rollback: failed.rollback,
            },
        },
    )
}
