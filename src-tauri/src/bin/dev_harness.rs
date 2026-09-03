//! A development-only way to drive the account loop without a user interface.
//!
//! **This is not a product feature and it is not in the release binary.** It is behind the
//! `dev-harness` Cargo feature and `required-features` in `Cargo.toml`, so `cargo build
//! --release` and the Tauri bundle never compile it, let alone ship it.
//!
//! Everything printed here goes through the same masking the rest of the application uses:
//! addresses are masked, account ids are the random internal ones, and no path, command line or
//! credential is ever printed. That is not politeness - a harness that printed more than the
//! app does would make the leak tests meaningless.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use toglet_lib::accounts::external_change::{self, ActiveAccount, ExitSync, ExternalChange};
use toglet_lib::accounts::onboarding::{LoginOutcome, PendingLogin};
use toglet_lib::accounts::{AccountIdentity, AccountProfile, mask_email, onboarding, repository};
use toglet_lib::app_server::{AppServerClient, AppServerSession, CodexBinary};
use toglet_lib::codex_home::{IsolatedHome, detect_environment};
use toglet_lib::credentials::{CredentialLock, CredentialRef, SecretStore};
use toglet_lib::diagnostics::{Phase, TogletError};
use toglet_lib::process::{ClientPresence, ClientProbe, SystemClientProbe};
use toglet_lib::quota::{NormalisedQuota, QuotaSnapshot};
use toglet_lib::storage::{MetadataDocument, MetadataStore};
use toglet_lib::switching::{
    Faults, NoObserver, Preflight, RollbackReport, Switch, SwitchLock, SwitchStage, SwitchTarget,
    adopt_current_session, read_default_identity, recover,
};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = arguments.first().map(String::as_str).unwrap_or("help");

    let result = match command {
        "detect" => detect(),
        "status" => status(),
        "probe" => probe(),
        "import" => import(arguments.get(1)),
        "login" => login(arguments.get(1)),
        "login-cancel" => login_cancel(),
        "remove" => remove(arguments.get(1)),
        "refresh" => refresh(arguments.get(1)),
        "switch" => switch(arguments.get(1), &arguments[1..]),
        "recover" => recovery(),
        "sync" => sync(arguments.get(1)),
        "whoami" => whoami(),
        _ => {
            help();
            return;
        }
    };

    if let Err(error) = result {
        // Only the code, the phase and the suggested action. The detail is deliberately not
        // printed: it can carry an operating-system message, and those carry paths.
        println!(
            "FAILED  code={}  phase={}  retryable={}  action={}",
            error.code().as_str(),
            error.phase().as_str(),
            error.retryable(),
            error.action().as_str()
        );
        // The redacted detail, which is what `diagnostics` allows to be recorded. It is printed
        // only here, where a developer is reading it, and never returned to a caller.
        println!("detail  {error:?}");
        std::process::exit(1);
    }
}

fn help() {
    println!(
        "dev_harness - development-only verification entry point (not a product feature)

  detect            run the seven first-run environment checks
  status            list stored accounts (masked) and the active one
  probe             list running Codex clients
  import <name>     import whoever the default Codex home is signed in as
  login <name>      sign in to another account in a throwaway home
  login-cancel      start a sign-in and cancel it part-way
  remove <id>       forget an account and delete its stored credentials
  refresh <id>      read one account's quota through a throwaway home
  switch <id>       run the pre-checks and switch to that account
  recover           run the start-up recovery for an interrupted switch
  sync [<id>]       run the final synchronisation that happens before exit
  whoami            ask a fresh app server who the default Codex home is signed in as

switch also accepts, for the fault-injection acceptance items:
  fail=write|replace|verify   make that stage fail, to watch the rollback
  halt=write|replace|verify   pause there and print the process id, so the switch can be
                              killed from Task Manager at a known point

Nothing here prints a token, a full address, a path, a command line or a sign-in URL."
    );
}

/// Where the harness keeps its metadata, kept apart from anything the real app would use.
fn data_directory() -> Result<PathBuf, TogletError> {
    let base = std::env::var_os("TOGLET_DEV_DATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Toglet")))
        .or_else(|| std::env::var_os("HOME").map(|dir| PathBuf::from(dir).join(".toglet")));
    let base = base.ok_or_else(|| internal("no application data directory could be determined"))?;

    ensure_private_dir(&base)?;
    Ok(base)
}

/// Creates the directory, or accepts an existing one **only after checking it is private**.
///
/// `create_private_dir` fails on an existing name on purpose - a temporary directory that is
/// already there may be somebody else's. An application data directory is different: it is
/// meant to survive between runs. Adopting it without checking its permissions would be the
/// mistake that rule exists to prevent, so it is checked.
fn ensure_private_dir(path: &std::path::Path) -> Result<(), TogletError> {
    match toglet_lib::codex_home::create_private_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            match toglet_lib::codex_home::is_private(path) {
                Ok(true) => Ok(()),
                Ok(false) => Err(internal("the data directory is readable by others")),
                Err(error) => Err(internal(&error.to_string())),
            }
        }
        Err(error) => Err(internal(&error.to_string())),
    }
}

fn codex_home() -> Result<PathBuf, TogletError> {
    if let Some(explicit) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(explicit));
    }
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .map(|home| PathBuf::from(home).join(".codex"))
        .ok_or_else(|| internal("the Codex home could not be determined"))
}

fn internal(detail: &str) -> TogletError {
    TogletError::new(
        toglet_lib::diagnostics::ErrorCode::Internal,
        Phase::Detect,
        false,
        toglet_lib::diagnostics::UserAction::None,
    )
    .with_detail(detail)
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

/// The platform credential store, rooted in the harness's own data directory.
fn store() -> Result<Box<dyn SecretStore>, TogletError> {
    let directory = data_directory()?.join("credentials");
    ensure_private_dir(&directory)?;

    #[cfg(windows)]
    {
        Ok(Box::new(toglet_lib::credentials::WindowsSecretStore::new(
            directory,
        )))
    }
    #[cfg(not(windows))]
    {
        let _ = directory;
        Err(internal(
            "the harness only wires the Windows credential store",
        ))
    }
}

fn detect() -> Result<(), TogletError> {
    for check in detect_environment().checks {
        println!(
            "{:<28} {:?}  {}",
            format!("{:?}", check.id),
            check.status,
            check.code.unwrap_or("-")
        );
    }
    Ok(())
}

fn load() -> Result<(MetadataStore, MetadataDocument), TogletError> {
    let directory = data_directory()?;
    let store = MetadataStore::new(&directory);
    let (document, outcome) = store.load();
    println!("metadata: {outcome:?}");
    Ok((store, document))
}

fn status() -> Result<(), TogletError> {
    let (_, document) = load()?;
    println!("active: {:?}", document.settings.active_account_id());
    println!("codexConfig: {:?}", document.codex_config);
    if document.accounts.is_empty() {
        println!("(no accounts)");
    }
    for account in &document.accounts {
        println!(
            "  {}  {:<24} {:?}  plan={:?}",
            account.id, account.display_name, account.status, account.plan_type
        );
    }
    Ok(())
}

fn probe() -> Result<(), TogletError> {
    match SystemClientProbe::new().running_clients(&[]) {
        ClientPresence::Unknown => println!("unknown - the probe could not run"),
        ClientPresence::Known(clients) if clients.is_empty() => println!("no Codex client running"),
        ClientPresence::Known(clients) => {
            for client in clients {
                // The executable path is not printed: it is an absolute path.
                println!("  pid={:<7} {:?}", client.pid, client.kind);
            }
        }
    }
    Ok(())
}

fn import(name: Option<&String>) -> Result<(), TogletError> {
    let name = name.map(String::as_str);
    let home = codex_home()?;
    let binary = CodexBinary::resolve(Phase::Detect)?;
    let (metadata, mut document) = load()?;

    let secret = onboarding::read_default_credentials(&home, Phase::Detect)?;
    let verified = onboarding::verify(
        &binary,
        IsolatedHome::create(Phase::Detect)?,
        secret,
        Phase::Detect,
    )?;
    println!("signed in as: {:?}", verified.masked_email());

    let adopted = adopt_current_session(&binary, &home, &verified).ok();

    let id = format!("acct-{}", now_seconds());
    let outcome = onboarding::adopt(
        store()?.as_ref(),
        &mut document,
        &verified,
        name,
        &id,
        &timestamp(),
    )?;

    let active = match &outcome {
        toglet_lib::accounts::fingerprint::DuplicateCheck::AlreadyPresent { existing_id } => {
            existing_id.clone()
        }
        toglet_lib::accounts::fingerprint::DuplicateCheck::New => id.clone(),
    };
    if let Some(token) = &adopted {
        document.settings.set_active_account_id(Some(active), token);
        println!("the default home is signed in as this account - recorded as active");
    }

    metadata.save(&document)?;
    println!("{outcome:?}");
    Ok(())
}

/// Signs in to a new account, entirely inside a throwaway home.
///
/// The default Codex home is not touched at any point: the sign-in happens in a directory that
/// deletes itself, and the credentials it produces are read out of that directory and encrypted.
fn login(name: Option<&String>) -> Result<(), TogletError> {
    let name = name.map(String::as_str);
    let binary = CodexBinary::resolve(Phase::Login)?;
    let (metadata, mut document) = load()?;

    let mut pending =
        PendingLogin::start(&binary, IsolatedHome::create(Phase::Login)?, Phase::Login)?;
    // Handed to the browser, never printed: it carries the PKCE challenge and the OAuth state.
    let opened = toglet_lib::process::open_url(pending.auth_url(), Phase::Login);
    if let Err(error) = opened {
        drop(pending.finish());
        return Err(error);
    }
    println!("a browser window was opened - finish the sign-in there (up to 5 minutes)");

    let outcome = pending.wait(onboarding::LOGIN_TIMEOUT);
    println!("login: {outcome:?}");
    if outcome != LoginOutcome::Completed {
        // The sign-in did not produce credentials, so there is nothing to store. Reported as
        // what it was rather than as a failure of some other kind.
        pending.finish()?;
        return Err(match outcome {
            LoginOutcome::Canceled => TogletError::new(
                toglet_lib::diagnostics::ErrorCode::LoginCanceled,
                Phase::Login,
                false,
                toglet_lib::diagnostics::UserAction::None,
            ),
            LoginOutcome::TimedOut => TogletError::new(
                toglet_lib::diagnostics::ErrorCode::LoginTimeout,
                Phase::Login,
                true,
                toglet_lib::diagnostics::UserAction::Retry,
            ),
            _ => internal("the sign-in did not complete"),
        });
    }

    // Read before `finish`, which drops the throwaway home and deletes it.
    let secret = pending.credentials(Phase::Login)?;
    pending.finish()?;

    let verified = onboarding::verify(
        &binary,
        IsolatedHome::create(Phase::Login)?,
        secret,
        Phase::Login,
    )?;
    println!("signed in as: {:?}", verified.masked_email());

    let id = format!("acct-{}", now_seconds());
    let outcome = onboarding::adopt(
        store()?.as_ref(),
        &mut document,
        &verified,
        name,
        &id,
        &timestamp(),
    )?;
    metadata.save(&document)?;
    println!("{outcome:?}");
    Ok(())
}

/// Starts a sign-in and cancels it part-way, to check the cleanup.
fn login_cancel() -> Result<(), TogletError> {
    const GRACE: std::time::Duration = std::time::Duration::from_secs(20);

    let binary = CodexBinary::resolve(Phase::Login)?;
    let mut pending =
        PendingLogin::start(&binary, IsolatedHome::create(Phase::Login)?, Phase::Login)?;
    if let Err(error) = toglet_lib::process::open_url(pending.auth_url(), Phase::Login) {
        drop(pending.finish());
        return Err(error);
    }

    println!("a browser window was opened. Do NOT finish the sign-in.");
    println!(
        "the sign-in will be cancelled in {} seconds",
        GRACE.as_secs()
    );
    std::thread::sleep(GRACE);

    pending.cancel()?;
    // Whether the server also notifies is not the point; what matters is that a cancellation
    // is reported as a cancellation rather than as a failure.
    println!("after cancelling: {:?}", pending.wait(GRACE));
    pending.finish()
}

/// Forgets an account and deletes its stored credentials.
fn remove(id: Option<&String>) -> Result<(), TogletError> {
    let id = id.ok_or_else(|| internal("an account id is required"))?;
    let (metadata, mut document) = load()?;

    let removed = onboarding::forget(store()?.as_ref(), &mut document, id)?;
    metadata.save(&document)?;
    println!("removed {} ({})", removed.id, removed.display_name);
    Ok(())
}

/// Failure injection for the manual switch items, supplied as a constructor argument exactly
/// the way the integration tests supply it. The release binary has no path to this.
struct HarnessFaults {
    fail_at: Option<SwitchStage>,
    halt_at: Option<SwitchStage>,
}

impl Faults for HarnessFaults {
    fn before(&self, stage: SwitchStage) -> Result<(), TogletError> {
        if self.halt_at == Some(stage) {
            const WINDOW: std::time::Duration = std::time::Duration::from_secs(180);
            println!(
                "HALTED before {:?} - process id {}. Kill it now from Task Manager to interrupt \
                 the switch at this exact point.",
                stage,
                std::process::id()
            );
            std::thread::sleep(WINDOW);
            println!(
                "nothing killed it within {} s - continuing",
                WINDOW.as_secs()
            );
        }

        if self.fail_at == Some(stage) {
            // The phase follows the stage so the printed result is not misleading about where
            // the switch actually stopped.
            let phase = match stage {
                SwitchStage::Write | SwitchStage::Replace => Phase::Write,
                SwitchStage::Verify => Phase::Verify,
            };
            return Err(TogletError::new(
                toglet_lib::diagnostics::ErrorCode::Internal,
                phase,
                false,
                toglet_lib::diagnostics::UserAction::Retry,
            )
            .with_detail("injected failure"));
        }
        Ok(())
    }
}

fn stage_argument(arguments: &[String], prefix: &str) -> Result<Option<SwitchStage>, TogletError> {
    let Some(value) = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix(prefix))
    else {
        return Ok(None);
    };

    match value {
        "write" => Ok(Some(SwitchStage::Write)),
        "replace" => Ok(Some(SwitchStage::Replace)),
        "verify" => Ok(Some(SwitchStage::Verify)),
        _ => Err(internal("the stage must be write, replace or verify")),
    }
}

fn refresh(id: Option<&String>) -> Result<(), TogletError> {
    let id = id.ok_or_else(|| internal("an account id is required"))?;
    let (_, document) = load()?;
    let account = account(&document, id)?;
    let binary = CodexBinary::resolve(Phase::ReadQuota)?;
    let reference = CredentialRef::new(&account.credential_ref)?;

    // The isolated home is the whole point: a quota read must not touch the default one.
    let home = IsolatedHome::create(Phase::ReadQuota)?;
    let secret = store()?.load(&reference)?;
    // `atomic_write` creates the file with the permissions already applied, which is what the
    // quota path does too.
    toglet_lib::codex_home::atomic_write(&home.path().join("auth.json"), secret.expose())
        .map_err(|error| internal(&error.to_string()))?;

    let mut session = AppServerSession::open(AppServerClient::start(&binary, home)?)?;
    let raw = session.read_rate_limits();
    session.close()?;

    let snapshot = QuotaSnapshot::fresh(id, NormalisedQuota::from_raw(&raw?), now_seconds());
    for window in &snapshot.quota().windows {
        println!(
            "  {:?}: used={:?}% remaining={:?}% resets_at={:?}",
            window.kind, window.used_percent, window.remaining_percent, window.resets_at
        );
    }
    if snapshot.quota().windows.is_empty() {
        println!("  (the server returned no windows - reported as unknown, never as 0%)");
    }
    Ok(())
}

fn switch(id: Option<&String>, arguments: &[String]) -> Result<(), TogletError> {
    let id = id.ok_or_else(|| internal("an account id is required"))?;
    let faults = HarnessFaults {
        fail_at: stage_argument(arguments, "fail=")?,
        halt_at: stage_argument(arguments, "halt=")?,
    };
    let home = codex_home()?;
    let binary = CodexBinary::resolve(Phase::Precheck)?;
    let (metadata, mut document) = load()?;
    let target = account(&document, id)?.clone();
    let secrets = store()?;

    let active = document
        .settings
        .active_account_id()
        .and_then(|active| repository::find(&document, active))
        .map(|profile| {
            (
                profile.credential_ref.clone(),
                profile.account_fingerprint.clone(),
            )
        });
    let active_reference = match &active {
        Some((reference, _)) => Some(CredentialRef::new(reference)?),
        None => None,
    };

    let lock = SwitchLock::new();
    let credential_lock = CredentialLock::new();
    let probe = SystemClientProbe::new();
    let preflight = Preflight {
        lock: &lock,
        credential_lock: &credential_lock,
        store: secrets.as_ref(),
        probe: &probe,
        binary: &binary,
        default_home: &home,
        own_processes: &[],
    };

    let passed = match preflight.run(
        document.settings.active_account_id(),
        match (&active_reference, &active) {
            (Some(reference), Some((_, fingerprint))) => Some(ActiveAccount {
                credentials: reference,
                fingerprint,
            }),
            _ => None,
        },
        SwitchTarget {
            account_id: &target.id,
            credentials: &CredentialRef::new(&target.credential_ref)?,
        },
    ) {
        Ok(passed) => passed,
        Err(failure) => {
            println!("pre-check stopped at {:?}", failure.step);
            return Err(failure.error);
        }
    };
    println!("pre-checks passed, clients: {:?}", passed.verdict);

    let switch = Switch {
        binary: &binary,
        default_home: &home,
        journal_directory: metadata
            .path()
            .parent()
            .ok_or_else(|| internal("the metadata file has no directory"))?,
        faults: &faults,
        observer: &NoObserver,
    };

    // The switch duration is measured from here: everything after the pre-checks, which is
    // what counts as "the switch".
    let started = std::time::Instant::now();
    let outcome = switch.run(
        passed,
        document.settings.active_account_id(),
        &target.id,
        &format!("dev-{}", now_seconds()),
        &timestamp(),
    );
    println!("switch took {} ms", started.elapsed().as_millis());

    match outcome {
        Ok(succeeded) => {
            document
                .settings
                .set_active_account_id(Some(target.id.clone()), &succeeded.verified);
            metadata.save(&document)?;
            println!("switched, progress step {}", succeeded.progress.number());
            Ok(())
        }
        Err(failed) => {
            match &failed.rollback {
                RollbackReport::Failed { backup } => {
                    // The one place a path is printed, and only to a person who has to act on
                    // it: the previous credentials are still in that file.
                    println!(
                        "ROLLBACK FAILED - restore this file by hand: {}",
                        backup.display()
                    );
                }
                other => println!("rolled back: {other:?}"),
            }
            println!("progress reached step {}", failed.progress.number());
            Err(failed.error)
        }
    }
}

fn recovery() -> Result<(), TogletError> {
    let home = codex_home()?;
    let binary = CodexBinary::resolve(Phase::Verify)?;
    let (metadata, _) = load()?;
    let directory = metadata
        .path()
        .parent()
        .ok_or_else(|| internal("the metadata file has no directory"))?;

    // No expected identity: without one, recovery rolls back rather than completing on an
    // assumption. Completing an interrupted switch belongs to the wired application, which can
    // resolve the target's identity from its own credentials.
    let outcome = recover(&binary, &home, directory, None)?;
    println!("{outcome:?}");
    Ok(())
}

fn account<'a>(
    document: &'a MetadataDocument,
    id: &str,
) -> Result<&'a AccountProfile, TogletError> {
    repository::find(document, id).ok_or_else(|| internal("no account with that id"))
}

fn timestamp() -> String {
    // Seconds since the epoch rather than a formatted date: the harness has no date formatter,
    // and inventing one here would duplicate something the application does not need yet.
    format!("{}", now_seconds())
}

/// The final synchronisation that happens before exit, so it can be exercised by hand.
fn sync(active: Option<&String>) -> Result<(), TogletError> {
    let home = codex_home()?;
    let secrets = store()?;
    let (_, document) = load()?;

    // Without an active account every foreign sign-in looks external, which makes the answer
    // useless. The account to compare against is taken from the document, or named.
    let active_id = active
        .map(String::as_str)
        .or_else(|| document.settings.active_account_id());
    let profile = match active_id {
        Some(id) => Some(account(&document, id)?),
        None => None,
    };
    let reference = match profile {
        Some(profile) => Some(CredentialRef::new(&profile.credential_ref)?),
        None => None,
    };

    let outcome = external_change::synchronise_before_exit(
        &CredentialLock::new(),
        secrets.as_ref(),
        match (&reference, profile) {
            (Some(credentials), Some(profile)) => Some(ActiveAccount {
                credentials,
                fingerprint: &profile.account_fingerprint,
            }),
            _ => None,
        },
        &home,
        Phase::Storage,
    );
    println!("{}", change_name(&outcome));
    Ok(())
}

/// The outcome as a name rather than as its `Debug` form.
///
/// `ExternalLogin` carries the fingerprint of whoever signed in, and a fingerprint is still an
/// account identifier - printing it is exactly what the redaction rules forbid.
/// The name is the whole of what a person needs here.
fn change_name(outcome: &ExitSync) -> &'static str {
    let change = match outcome {
        ExitSync::Done(change) => change,
        ExitSync::Failed(code) => return code.as_str(),
    };
    match change {
        ExternalChange::Unchanged => "Unchanged",
        ExternalChange::SnapshotUpdated => "SnapshotUpdated",
        ExternalChange::NotUnderstood => "NotUnderstood",
        ExternalChange::ExternalLogin { .. } => "ExternalLogin",
        ExternalChange::SignedOut => "SignedOut",
    }
}

/// Who the **default** Codex home is signed in as, read by a freshly started app server.
///
/// This is what a new Codex session sees. It runs against the real home rather than a
/// throwaway one, and reads only - `ServerHome::Default` carries no write path.
fn whoami() -> Result<(), TogletError> {
    let home = codex_home()?;
    let binary = CodexBinary::resolve(Phase::Verify)?;

    match read_default_identity(&binary, &home, Phase::Verify)? {
        None => println!("the default Codex home is not signed in to any account"),
        Some(AccountIdentity::ApiKey) => println!("signed in with an API key"),
        Some(AccountIdentity::Chatgpt { email, plan_type }) => println!(
            "default home is signed in as {:?}  plan={:?}",
            mask_email(&email),
            plan_type
        ),
    }
    Ok(())
}
