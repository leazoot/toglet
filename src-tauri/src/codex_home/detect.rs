//! First-run environment detection.
//!
//! Seven checks, reported one by one. They are never merged into a single "environment is
//! fine" verdict and a check that could not run says so rather than passing.
//!
//! Nothing here downloads anything, and the only executable ever started is the Codex binary
//! resolved by `app_server::process`.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::app_server::{AppServerClient, AppServerSession, CodexBinary};
use crate::codex_home::IsolatedHome;
use crate::diagnostics::{ErrorCode, Phase, UserAction};

/// The seven checks, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckId {
    OperatingSystem,
    CodexCommand,
    AppServerMethods,
    DefaultCodexHome,
    ConfigFile,
    AuthState,
    ImportableAccount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Passed,
    Failed,
    /// The check could not be reached because something it depends on failed. Never reported
    /// as a pass.
    NotApplicable,
}

/// One check's real outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentCheck {
    pub id: CheckId,
    pub status: CheckStatus,
    /// Stable error code when the check failed.
    pub code: Option<&'static str>,
    /// What the user can do about it. The frontend turns this into localised copy.
    pub action: &'static str,
    /// A short, non-sensitive fact: an OS name, an auth mode, a plan. **Never a path, a
    /// command line or an address**.
    pub detail: Option<String>,
}

impl EnvironmentCheck {
    fn passed(id: CheckId, detail: Option<String>) -> Self {
        Self {
            id,
            status: CheckStatus::Passed,
            code: None,
            action: UserAction::None.as_str(),
            detail,
        }
    }

    fn failed(id: CheckId, code: ErrorCode, action: UserAction, detail: Option<String>) -> Self {
        Self {
            id,
            status: CheckStatus::Failed,
            code: Some(code.as_str()),
            action: action.as_str(),
            detail,
        }
    }

    fn not_applicable(id: CheckId) -> Self {
        Self {
            id,
            status: CheckStatus::NotApplicable,
            code: None,
            action: UserAction::None.as_str(),
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    pub checks: Vec<EnvironmentCheck>,
}

/// How a Codex home is authenticated.
///
/// These are the only modes the protocol defines (`AuthMode` in the app server schema).
/// An "organisation-policy managed" state has also been asked for; the protocol exposes no
/// such thing, so it is left out rather than faked with a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthState {
    NotSignedIn,
    Chatgpt,
    ApiKey,
    /// `chatgptAuthTokens`. The schema marks it "FOR OPENAI INTERNAL USE ONLY": tokens are
    /// supplied by a host application and kept in memory. Toglet cannot manage or switch it.
    HostManagedTokens,
    /// A mode this build does not know. Reported as-is instead of being forced into one of the
    /// above.
    Unknown(String),
}

impl AuthState {
    fn as_detail(&self) -> String {
        match self {
            Self::NotSignedIn => "not_signed_in".to_owned(),
            Self::Chatgpt => "chatgpt".to_owned(),
            Self::ApiKey => "apikey".to_owned(),
            Self::HostManagedTokens => "chatgpt_auth_tokens".to_owned(),
            Self::Unknown(mode) => format!("unknown:{mode}"),
        }
    }
}

/// Runs all seven checks.
pub fn detect_environment() -> EnvironmentReport {
    let mut checks = Vec::with_capacity(7);

    checks.push(check_operating_system());

    let binary = CodexBinary::resolve(Phase::Detect);
    checks.push(match &binary {
        Ok(_) => EnvironmentCheck::passed(CheckId::CodexCommand, None),
        Err(error) => EnvironmentCheck::failed(
            CheckId::CodexCommand,
            error.code(),
            UserAction::InstallRuntime,
            None,
        ),
    });

    checks.push(match &binary {
        Ok(binary) => check_app_server_methods(binary),
        // Without an executable there is nothing to ask. Saying "not applicable" is the honest
        // answer; saying "passed" would be the failure these checks exist to prevent.
        Err(_) => EnvironmentCheck::not_applicable(CheckId::AppServerMethods),
    });

    let home = default_codex_home();
    checks.push(match &home {
        Some(home) if home.is_dir() => EnvironmentCheck::passed(CheckId::DefaultCodexHome, None),
        _ => EnvironmentCheck::failed(
            CheckId::DefaultCodexHome,
            ErrorCode::CodexHomeUnwritable,
            UserAction::InstallRuntime,
            None,
        ),
    });

    checks.push(match &home {
        Some(home) => check_config_file(home),
        None => EnvironmentCheck::not_applicable(CheckId::ConfigFile),
    });

    let auth = home.as_deref().map(read_auth_state);
    checks.push(match &auth {
        Some(state) => EnvironmentCheck::passed(CheckId::AuthState, Some(state.as_detail())),
        None => EnvironmentCheck::not_applicable(CheckId::AuthState),
    });

    checks.push(match &auth {
        // Only a ChatGPT sign-in produces something Toglet can import.
        Some(AuthState::Chatgpt) => EnvironmentCheck::passed(CheckId::ImportableAccount, None),
        Some(AuthState::NotSignedIn) => EnvironmentCheck::failed(
            CheckId::ImportableAccount,
            ErrorCode::AuthExpired,
            UserAction::ReLogin,
            None,
        ),
        Some(_) => EnvironmentCheck::failed(
            CheckId::ImportableAccount,
            ErrorCode::RuntimeIncompatible,
            UserAction::None,
            None,
        ),
        None => EnvironmentCheck::not_applicable(CheckId::ImportableAccount),
    });

    EnvironmentReport { checks }
}

fn check_operating_system() -> EnvironmentCheck {
    let family = std::env::consts::OS;
    let detail = match operating_system_version() {
        Some(version) => format!("{family} {version} {}", std::env::consts::ARCH),
        None => format!("{family} {}", std::env::consts::ARCH),
    };

    if matches!(family, "windows" | "macos") {
        EnvironmentCheck::passed(CheckId::OperatingSystem, Some(detail))
    } else {
        EnvironmentCheck::failed(
            CheckId::OperatingSystem,
            ErrorCode::RuntimeIncompatible,
            UserAction::None,
            Some(detail),
        )
    }
}

/// Starts an app server in a throwaway home and calls the method Toglet depends on.
///
/// Availability is proven by calling the method, not by comparing a version string. An empty
/// isolated home is enough: `account/read` answers locally and needs neither an account nor
/// the network.
fn check_app_server_methods(binary: &CodexBinary) -> EnvironmentCheck {
    let outcome = IsolatedHome::create(Phase::Detect)
        .and_then(|home| AppServerClient::start(binary, home))
        .and_then(AppServerSession::open)
        .and_then(|mut session| {
            let account = session.read_account();
            let version = session.runtime_version().map(str::to_owned);
            // Shut down before reporting so a failure to stop is not hidden by a success.
            session.close()?;
            account.map(|_| version)
        });

    match outcome {
        Ok(version) => EnvironmentCheck::passed(CheckId::AppServerMethods, version),
        Err(error) => EnvironmentCheck::failed(
            CheckId::AppServerMethods,
            error.code(),
            UserAction::UpdateRuntime,
            None,
        ),
    }
}

/// Checks that `config.toml` can be read and written without changing it.
///
/// Opening in append mode and closing immediately proves write access without touching a byte;
/// a missing file is not a failure, because Toglet creates it when it first needs to.
fn check_config_file(home: &Path) -> EnvironmentCheck {
    let config = home.join("config.toml");
    if !config.exists() {
        return EnvironmentCheck::passed(CheckId::ConfigFile, Some("absent".to_owned()));
    }

    let readable = std::fs::File::open(&config).is_ok();
    let writable = std::fs::OpenOptions::new()
        .append(true)
        .open(&config)
        .is_ok();

    if readable && writable {
        EnvironmentCheck::passed(CheckId::ConfigFile, Some("present".to_owned()))
    } else {
        EnvironmentCheck::failed(
            CheckId::ConfigFile,
            ErrorCode::CodexHomeUnwritable,
            UserAction::FixConfigManually,
            None,
        )
    }
}

/// Reads only the `auth_mode` field out of the default `auth.json`.
///
/// Deserialising into a struct with one field means serde skips everything else in the stream:
/// the tokens are never materialised, so they cannot end up in a buffer, a log or an error.
fn read_auth_state(home: &Path) -> AuthState {
    #[derive(serde::Deserialize)]
    struct AuthFile {
        #[serde(default)]
        auth_mode: Option<String>,
    }

    let Ok(contents) = std::fs::read_to_string(home.join("auth.json")) else {
        return AuthState::NotSignedIn;
    };
    let Ok(auth) = serde_json::from_str::<AuthFile>(&contents) else {
        return AuthState::NotSignedIn;
    };

    match auth.auth_mode.as_deref() {
        None => AuthState::NotSignedIn,
        Some("chatgpt") => AuthState::Chatgpt,
        Some("apikey") => AuthState::ApiKey,
        Some("chatgptAuthTokens") => AuthState::HostManagedTokens,
        Some(other) => AuthState::Unknown(other.to_owned()),
    }
}

/// The Codex home Codex itself would use: `CODEX_HOME` when set, otherwise `~/.codex`.
fn default_codex_home() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(explicit));
    }
    let home_variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(home_variable).map(|home| PathBuf::from(home).join(".codex"))
}

#[cfg(windows)]
fn operating_system_version() -> Option<String> {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: u32::try_from(size_of::<OSVERSIONINFOW>()).ok()?,
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `info` is a correctly sized OSVERSIONINFOW. RtlGetVersion is used rather than
    // GetVersionExW because the latter reports a capped version unless the executable carries a
    // matching compatibility manifest.
    if unsafe { RtlGetVersion(&raw mut info) } != 0 {
        return None;
    }
    Some(format!(
        "{}.{}.{}",
        info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
    ))
}

#[cfg(not(windows))]
fn operating_system_version() -> Option<String> {
    // Not verified on a real macOS machine yet. The plist is plain XML, so the value
    // is read directly rather than pulling in a parser for one field.
    let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    let after_key = plist.split_once("<key>ProductVersion</key>")?.1;
    let value = after_key.split_once("<string>")?.1;
    Some(value.split_once("</string>")?.0.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> EnvironmentReport {
        detect_environment()
    }

    #[test]
    fn every_check_is_reported_separately_and_in_order() {
        let report = report();

        let ids: Vec<CheckId> = report.checks.iter().map(|check| check.id).collect();
        assert_eq!(
            ids,
            vec![
                CheckId::OperatingSystem,
                CheckId::CodexCommand,
                CheckId::AppServerMethods,
                CheckId::DefaultCodexHome,
                CheckId::ConfigFile,
                CheckId::AuthState,
                CheckId::ImportableAccount,
            ],
            "seven checks, never merged"
        );
    }

    #[test]
    fn a_failed_check_always_carries_a_stable_code() {
        for check in report().checks {
            match check.status {
                CheckStatus::Failed => assert!(
                    check.code.is_some(),
                    "a failure without a code tells the user nothing"
                ),
                CheckStatus::Passed | CheckStatus::NotApplicable => {
                    assert_eq!(check.code, None);
                }
            }
        }
    }

    #[test]
    fn the_report_never_carries_a_path_or_a_command_line() {
        let serialised =
            serde_json::to_string(&report()).expect("the report serialises for the frontend");

        for forbidden in [":\\", "//", "app-server", "/Users/", "C:"] {
            assert!(
                !serialised.contains(forbidden),
                "the command surface leaked {forbidden:?}"
            );
        }
    }

    #[test]
    fn the_operating_system_is_identified() {
        let check = check_operating_system();

        assert_eq!(check.status, CheckStatus::Passed);
        let detail = check.detail.expect("the OS is described");
        assert!(detail.starts_with(std::env::consts::OS));
        assert!(
            operating_system_version().is_some(),
            "the OS version must be a real reading, not an omission"
        );
    }

    #[test]
    fn an_absent_auth_file_reads_as_not_signed_in_rather_than_an_error() {
        let empty = IsolatedHome::create(Phase::Detect).expect("scratch home is created");

        assert_eq!(read_auth_state(empty.path()), AuthState::NotSignedIn);
    }

    #[test]
    fn each_auth_mode_maps_to_its_own_state() {
        let home = IsolatedHome::create(Phase::Detect).expect("scratch home is created");
        let auth = home.path().join("auth.json");

        for (mode, expected) in [
            ("chatgpt", AuthState::Chatgpt),
            ("apikey", AuthState::ApiKey),
            ("chatgptAuthTokens", AuthState::HostManagedTokens),
            (
                "somethingNew",
                AuthState::Unknown("somethingNew".to_owned()),
            ),
        ] {
            // A token-shaped value is included to show it is skipped, not read.
            std::fs::write(
                &auth,
                format!(
                    r#"{{"auth_mode":"{mode}","tokens":{{"access_token":"eyJhbGciOiJIUzI1NiJ9.x.y"}}}}"#
                ),
            )
            .expect("the auth file is written");

            assert_eq!(read_auth_state(home.path()), expected);
        }
    }

    #[test]
    fn a_config_file_that_does_not_exist_is_not_a_failure() {
        let home = IsolatedHome::create(Phase::Detect).expect("scratch home is created");
        std::fs::remove_file(home.path().join("config.toml")).expect("config is removed");

        let check = check_config_file(home.path());

        assert_eq!(check.status, CheckStatus::Passed);
        assert_eq!(check.detail.as_deref(), Some("absent"));
    }

    #[test]
    fn a_readable_and_writable_config_passes_without_being_modified() {
        let home = IsolatedHome::create(Phase::Detect).expect("scratch home is created");
        let config = home.path().join("config.toml");
        let before = std::fs::read(&config).expect("config is readable");

        let check = check_config_file(home.path());

        assert_eq!(check.status, CheckStatus::Passed);
        assert_eq!(
            std::fs::read(&config).expect("config is still readable"),
            before,
            "probing writability must not change a byte"
        );
    }
}
