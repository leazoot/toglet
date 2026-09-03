//! The wire shapes `codex app-server` actually returns, and their conversion to domain types.
//!
//! Recorded from a real server, not from guesswork. This is the one module expected to change
//! when the official interface changes.
//!
//! Two rules govern every type here:
//!
//! * **Unknown fields are ignored**, which is serde's default and is left that way on purpose:
//!   a field Toglet has never heard of must not break a quota read.
//! * **Missing required fields are an error**, never a default. `usedPercent` absent means the
//!   server did not tell us the usage - it does not mean zero.

use serde::Deserialize;

use crate::accounts::AccountIdentity;

/// `initialize` result.
///
/// Observed on a real server: this carries a single field. There is no protocol version and no
/// server capability list, so the version has to be read out of the user agent string.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeResult {
    pub user_agent: String,
}

/// `account/read` result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountReadResult {
    /// `null` means nobody is signed in (verified against an empty isolated home).
    pub account: Option<AccountDto>,
    // `requiresOpenaiAuth` is deliberately not modelled. It was observed to stay `true` after
    // a successful login, so reading it as a sign-in signal would be wrong.
}

/// The tagged union the server uses for an account.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum AccountDto {
    ApiKey,
    // `rename_all` on the enum renames the *variants*; a variant's fields need their own
    // attribute. Without this, `planType` silently deserialises to `None` and an account on a
    // paid plan is reported as having an unknown one.
    #[serde(rename_all = "camelCase")]
    Chatgpt {
        email: String,
        #[serde(default)]
        plan_type: Option<String>,
    },
}

impl From<AccountDto> for AccountIdentity {
    fn from(dto: AccountDto) -> Self {
        match dto {
            AccountDto::ApiKey => Self::ApiKey,
            AccountDto::Chatgpt { email, plan_type } => Self::Chatgpt {
                email,
                // The server's own `"unknown"` is an absence of information, so it becomes
                // `None` rather than being stored as the literal word.
                plan_type: plan_type.filter(|plan| plan != "unknown"),
            },
        }
    }
}

/// `account/login/start` result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginStartResult {
    pub login_id: String,
    /// The OAuth URL the user opens. Never logged: it carries PKCE parameters, and `redact`
    /// removes whole URLs precisely because of values like this.
    pub auth_url: String,
}

/// `account/login/cancel` result. `status` is `canceled` or `notFound`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginCancelResult {
    pub status: String,
}

/// The `account/login/completed` notification.
///
/// Observed on a real server: **a user cancelling and a genuine failure are indistinguishable
/// here** - both arrive as `success: false`. Telling them apart is the caller's job, by
/// remembering that it asked for a cancellation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginCompletedParams {
    pub login_id: String,
    pub success: bool,
}

/// `account/rateLimits/read` result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimitsResult {
    pub rate_limits: RateLimitsDto,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimitsDto {
    #[serde(default)]
    pub primary: Option<WindowDto>,
    #[serde(default)]
    pub secondary: Option<WindowDto>,
    #[serde(default)]
    pub plan_type: Option<String>,
    // `credits` is not modelled: it is out of scope, and serde's ignoring of unknown fields
    // tolerates it without Toglet pretending to understand it.
}

/// One rate limit window exactly as returned.
///
/// Observed on a real server: only `usedPercent` is required. Both other fields are genuinely
/// nullable, and a `null` `windowDurationMins` means the window type is **unknown** - it must
/// not be inferred from whether the window arrived as `primary` or `secondary`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowDto {
    /// Read as a float even though an `int32` was measured. An integer deserialises into `f64`
    /// without loss, and if the server ever reports a fraction it is carried through instead of
    /// being rejected. Rounding is the display layer's decision, not this one's.
    pub used_percent: f64,
    #[serde(default)]
    pub window_duration_mins: Option<i64>,
    #[serde(default)]
    pub resets_at: Option<i64>,
}

/// Quota data as the server gave it, with nothing classified or filled in.
///
/// Turning windows into five-hour and weekly buckets belongs to `quota`, which owns the single
/// implementation of that rule. Doing it here would create a second one.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRateLimits {
    pub primary: Option<RawWindow>,
    pub secondary: Option<RawWindow>,
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawWindow {
    /// Exactly what the server reported. Not clamped, not rounded - `quota::normalize` owns
    /// both of those decisions.
    pub used_percent: f64,
    /// `None` when the server did not say. The window type is then unknown, not five-hour.
    pub window_duration_mins: Option<i64>,
    /// Unix seconds. `None` when the server did not say.
    pub resets_at: Option<i64>,
}

impl From<WindowDto> for RawWindow {
    fn from(dto: WindowDto) -> Self {
        Self {
            used_percent: dto.used_percent,
            window_duration_mins: dto.window_duration_mins,
            resets_at: dto.resets_at,
        }
    }
}

impl From<RateLimitsDto> for RawRateLimits {
    fn from(dto: RateLimitsDto) -> Self {
        Self {
            primary: dto.primary.map(RawWindow::from),
            secondary: dto.secondary.map(RawWindow::from),
            plan_type: dto.plan_type.filter(|plan| plan != "unknown"),
        }
    }
}

/// `configRequirements/read` result.
///
/// `requirements: null` means no organisation-enforced configuration is present - measured on
/// this machine. The contents are deliberately **not** modelled: Toglet only needs to know
/// whether any exist, and modelling a structure that has never been observed would be
/// guesswork.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigRequirementsResult {
    #[serde(default)]
    pub requirements: Option<serde_json::Value>,
}

/// `config/read` result.
///
/// **Only the one key Toglet manages is modelled.** The real response carries the fully merged
/// configuration - provider base URLs, API-key environment variable names, `notify` command
/// paths and every trusted project path on the machine. Serde ignores what it is not asked
/// for, so none of that is ever materialised into a buffer that could reach a log or an error
/// Widening this struct means taking on that risk deliberately.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigReadResult {
    #[serde(default)]
    pub config: ConfigValues,
    /// Per-key provenance: which layer supplied the value, and that layer's version token.
    #[serde(default)]
    pub origins: std::collections::BTreeMap<String, ConfigOrigin>,
}

/// Configuration values, which are **snake_case on the wire** unlike the rest of the protocol.
///
/// No `rename_all` here on purpose: these are TOML keys passed through verbatim, not protocol
/// fields. Adding the camelCase rename the neighbouring types use makes the key silently
/// deserialise to `None`, so Toglet concludes the setting is unset, writes it, and then fails
/// its own read-back. Caught by a test, not by review - the same trap `AccountDto` documents.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConfigValues {
    #[serde(default)]
    pub cli_auth_credentials_store: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigOrigin {
    pub name: ConfigLayer,
    /// Opaque token identifying the layer's current content. Measured to be **neither** the
    /// file's SHA-256 nor the pre-write content's, so it is passed back verbatim and never
    /// recomputed.
    #[serde(default)]
    pub version: Option<String>,
}

impl ConfigOrigin {
    /// Whether this value comes from the user's own `config.toml`, the only layer Toglet writes.
    pub fn layer_type_is_user(&self) -> bool {
        self.name.layer_type == USER_CONFIG_LAYER
    }
}

/// Which configuration layer a value came from.
///
/// The `file` field is deliberately not modelled - it is an absolute path.
#[derive(Debug, Deserialize)]
pub(crate) struct ConfigLayer {
    #[serde(rename = "type")]
    pub layer_type: String,
}

/// The layer Toglet is allowed to write to: the user's own `config.toml`.
///
/// Anything else - `legacyManagedConfigTomlFromMdm`, `legacyManagedConfigTomlFromFile`,
/// `sessionFlags`, `dotCodexFolder`, or a type that does not exist yet - is treated as
/// not-ours. An unrecognised layer stops the write rather than being assumed harmless, which
/// is the only safe default when the value may be enforced by an organisation.
pub(crate) const USER_CONFIG_LAYER: &str = "user";

/// `config/value/write` result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigWriteResult {
    /// The layer's new version token, to be presented as `expectedVersion` on the next write.
    #[serde(default)]
    pub version: Option<String>,
    /// Non-null when a higher-priority layer overrides what was just written - the value is in
    /// the file but has no effect. Reported honestly rather than as a success.
    #[serde(default)]
    pub overridden_metadata: Option<serde_json::Value>,
    // `filePath` is deliberately not modelled: it is an absolute path, which must not enter
    // logs or error details.
}

/// The configuration key that selects how Codex stores credentials.
pub const CREDENTIAL_STORE_KEY: &str = "cli_auth_credentials_store";

/// The value Toglet needs: credentials in `auth.json` rather than the OS credential store.
pub const CREDENTIAL_STORE_FILE: &str = "file";

/// What the current configuration says about the credential store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialStoreSetting {
    /// `None` when no layer sets the key at all.
    pub value: Option<String>,
    /// `Some(true)` when the user's own `config.toml` supplies it, `Some(false)` when some
    /// other layer does, `None` when nothing does yet. `Some(false)` is the case Toglet must
    /// not write through.
    pub written_by_user_layer: Option<bool>,
    /// Opaque token to present as `expectedVersion` on the next write.
    pub version: Option<String>,
}

impl CredentialStoreSetting {
    /// Whether Codex is already storing credentials in the file Toglet manages.
    pub fn is_file_mode(&self) -> bool {
        self.value.as_deref() == Some(CREDENTIAL_STORE_FILE)
    }

    /// Whether some layer other than the user's own config supplies the value.
    pub fn is_externally_managed(&self) -> bool {
        self.written_by_user_layer == Some(false)
    }
}

/// What a successful configuration write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWriteOutcome {
    /// The token to present on the next write.
    pub version: Option<String>,
    /// The value was written but a higher-priority layer overrides it, so it has no effect.
    /// A write that lands without taking effect is a failure to report, not a success.
    pub overridden: bool,
}

/// A server-defined `config/value/write` failure, carried in the JSON-RPC error's `data`.
///
/// The six values were read out of the runtime binary and one of them (`configVersionConflict`)
/// was reproduced against a live server.
#[derive(Debug, Deserialize)]
pub(crate) struct ConfigWriteErrorData {
    #[serde(default)]
    pub config_write_error_code: Option<String>,
}

/// The Codex version, read out of the user agent (`<name>/0.98.0 (...)`).
///
/// Best effort by design: the format is not a documented contract, so a version that cannot be
/// parsed yields `None` and is reported as unknown rather than being invented. Nothing gates
/// on it - see the compatibility note in `wire`.
pub(crate) fn runtime_version(user_agent: &str) -> Option<String> {
    let after_slash = user_agent.split_once('/')?.1;
    let version = after_slash
        .split_whitespace()
        .next()
        .filter(|candidate| candidate.starts_with(|c: char| c.is_ascii_digit()))?;
    Some(version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn a_real_rate_limits_payload_round_trips() {
        // The exact body recorded from a real server.
        let result: RateLimitsResult = parse(
            r#"{"rateLimits":{"primary":{"usedPercent":2,"windowDurationMins":300,"resetsAt":1788164992},
                "secondary":{"usedPercent":0,"windowDurationMins":10080,"resetsAt":1788751792},
                "credits":{"hasCredits":false,"unlimited":false,"balance":"0"},"planType":"plus"}}"#,
        )
        .expect("the recorded payload parses");

        let limits = RawRateLimits::from(result.rate_limits);
        assert_eq!(
            limits.primary,
            Some(RawWindow {
                used_percent: 2.0,
                window_duration_mins: Some(300),
                resets_at: Some(1_788_164_992),
            })
        );
        assert_eq!(limits.plan_type.as_deref(), Some("plus"));
    }

    #[test]
    fn an_unknown_field_is_ignored_rather_than_rejected() {
        let result: RateLimitsResult = parse(
            r#"{"rateLimits":{"primary":{"usedPercent":5,"somethingNew":42},"anotherNewThing":true}}"#,
        )
        .expect("a payload with new fields still parses");

        let limits = RawRateLimits::from(result.rate_limits);
        assert!(
            (limits.primary.expect("primary is present").used_percent - 5.0).abs() < f64::EPSILON
        );
    }

    #[test]
    fn a_missing_window_stays_missing_and_is_never_zero() {
        let result: RateLimitsResult =
            parse(r#"{"rateLimits":{"primary":{"usedPercent":7}}}"#).expect("payload parses");

        let limits = RawRateLimits::from(result.rate_limits);
        // The weekly window was not returned. That is not "0% used".
        assert_eq!(limits.secondary, None);
        let primary = limits.primary.expect("primary is present");
        assert!((primary.used_percent - 7.0).abs() < f64::EPSILON);
        // Nor is an absent duration a five-hour window.
        assert_eq!(primary.window_duration_mins, None);
        assert_eq!(primary.resets_at, None);
    }

    #[test]
    fn a_window_without_used_percent_is_an_error_not_a_default() {
        let parsed: Result<RateLimitsResult, _> =
            parse(r#"{"rateLimits":{"primary":{"windowDurationMins":300}}}"#);

        assert!(
            parsed.is_err(),
            "a missing usedPercent must not become 0 - that is a red line"
        );
    }

    #[test]
    fn a_chatgpt_account_maps_to_the_domain_type() {
        let result: AccountReadResult =
            parse(r#"{"account":{"type":"chatgpt","email":"a@b.com","planType":"plus"},"requiresOpenaiAuth":true}"#)
                .expect("payload parses");

        let identity = AccountIdentity::from(result.account.expect("an account is present"));
        assert_eq!(
            identity,
            AccountIdentity::Chatgpt {
                email: "a@b.com".to_owned(),
                plan_type: Some("plus".to_owned()),
            }
        );
    }

    #[test]
    fn a_server_reported_unknown_plan_becomes_none() {
        let result: AccountReadResult =
            parse(r#"{"account":{"type":"chatgpt","email":"a@b.com","planType":"unknown"}}"#)
                .expect("payload parses");

        let identity = AccountIdentity::from(result.account.expect("an account is present"));
        assert_eq!(
            identity,
            AccountIdentity::Chatgpt {
                email: "a@b.com".to_owned(),
                plan_type: None,
            }
        );
    }

    #[test]
    fn a_chatgpt_account_without_an_email_is_rejected() {
        let parsed: Result<AccountReadResult, _> =
            parse(r#"{"account":{"type":"chatgpt","planType":"plus"}}"#);

        assert!(parsed.is_err(), "email is required for a ChatGPT account");
    }

    #[test]
    fn a_null_account_means_nobody_is_signed_in() {
        let result: AccountReadResult =
            parse(r#"{"account":null,"requiresOpenaiAuth":true}"#).expect("payload parses");

        assert!(result.account.is_none());
    }

    #[test]
    fn an_api_key_account_is_recognised() {
        let result: AccountReadResult =
            parse(r#"{"account":{"type":"apiKey"}}"#).expect("payload parses");

        let identity = AccountIdentity::from(result.account.expect("an account is present"));
        assert_eq!(identity, AccountIdentity::ApiKey);
        assert!(!identity.is_manageable());
    }

    #[test]
    fn initialize_requires_a_user_agent() {
        assert!(parse::<InitializeResult>(r#"{}"#).is_err());
        let result: InitializeResult =
            parse(r#"{"userAgent":"toglet/0.98.0 (Windows)","extra":1}"#).expect("payload parses");
        assert_eq!(result.user_agent, "toglet/0.98.0 (Windows)");
    }

    #[test]
    fn the_runtime_version_is_read_from_the_user_agent() {
        assert_eq!(
            runtime_version("toglet/0.98.0 (Windows 10.0.26200; x86_64) WindowsTerminal")
                .as_deref(),
            Some("0.98.0")
        );
    }

    #[test]
    fn an_unparseable_user_agent_yields_unknown_rather_than_a_guess() {
        assert_eq!(runtime_version("no-slash-here"), None);
        assert_eq!(runtime_version("name/notaversion"), None);
    }

    #[test]
    fn no_organisation_requirements_reads_as_none() {
        // Measured on this machine: `{"requirements":null}`.
        let result: ConfigRequirementsResult =
            parse(r#"{"requirements":null}"#).expect("payload parses");

        assert!(result.requirements.is_none());
    }

    #[test]
    fn present_requirements_are_detected_without_being_interpreted() {
        let result: ConfigRequirementsResult =
            parse(r#"{"requirements":{"allowedApprovalPolicies":["never"]}}"#)
                .expect("payload parses");

        assert!(
            result.requirements.is_some(),
            "an enforced configuration must be visible even though its shape is unknown"
        );
    }

    #[test]
    fn reading_the_config_captures_the_managed_key_and_its_origin() {
        // The response shape recorded from a live server.
        let result: ConfigReadResult = parse(
            r#"{"config":{"cli_auth_credentials_store":"file","model":"gpt-5.6-sol"},
                "origins":{"cli_auth_credentials_store":{"name":{"type":"user","file":"C:\\x\\config.toml"},
                "version":"sha256:abc"}}}"#,
        )
        .expect("payload parses");

        assert_eq!(
            result.config.cli_auth_credentials_store.as_deref(),
            Some("file")
        );
        let origin = result
            .origins
            .get("cli_auth_credentials_store")
            .expect("the origin is present");
        assert!(origin.layer_type_is_user());
        assert_eq!(origin.version.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn a_managed_layer_is_not_mistaken_for_the_user_layer() {
        let result: ConfigReadResult = parse(
            r#"{"config":{},"origins":{"cli_auth_credentials_store":
                {"name":{"type":"legacyManagedConfigTomlFromMdm"},"version":"sha256:abc"}}}"#,
        )
        .expect("payload parses");

        assert!(
            !result.origins["cli_auth_credentials_store"].layer_type_is_user(),
            "an MDM layer must not be treated as writable"
        );
    }

    #[test]
    fn an_unknown_layer_type_is_treated_as_not_ours() {
        let result: ConfigReadResult = parse(
            r#"{"config":{},"origins":{"cli_auth_credentials_store":
                {"name":{"type":"somethingInventedLater"}}}}"#,
        )
        .expect("payload parses");

        assert!(
            !result.origins["cli_auth_credentials_store"].layer_type_is_user(),
            "a layer Toglet has never heard of must stop the write, not be assumed writable"
        );
    }

    #[test]
    fn reading_the_config_does_not_materialise_provider_urls_or_project_paths() {
        // A trimmed copy of a real response. Everything here except the one managed key is
        // sensitive: endpoints, credential environment variable names, and every trusted
        // project path on the machine.
        let json = r#"{"config":{"cli_auth_credentials_store":"file",
            "model_providers":{"p":{"base_url":"https://secret.example/v1","env_key":"SECRET_KEY"}},
            "projects":{"c:\\users\\someone\\private":{"trust_level":"trusted"}},
            "notify":["C:\\Users\\someone\\AppData\\hook.exe"]},"origins":{}}"#;

        let result: ConfigReadResult = parse(json).expect("payload parses");

        // The struct has exactly one field, so there is nowhere for the rest to be kept.
        let captured = format!("{result:?}");
        for secret in [
            "secret.example",
            "SECRET_KEY",
            "private",
            "hook.exe",
            "trusted",
        ] {
            assert!(
                !captured.contains(secret),
                "`{secret}` must never be materialised by a config read"
            );
        }
        assert_eq!(
            result.config.cli_auth_credentials_store.as_deref(),
            Some("file")
        );
    }

    #[test]
    fn a_write_result_carries_the_next_version_and_no_file_path() {
        // Recorded from a live server.
        let result: ConfigWriteResult = parse(
            r#"{"status":"ok","version":"sha256:d13ff0df","filePath":"\\\\?\\C:\\Users\\x\\config.toml",
                "overriddenMetadata":null}"#,
        )
        .expect("payload parses");

        assert_eq!(result.version.as_deref(), Some("sha256:d13ff0df"));
        assert!(result.overridden_metadata.is_none());
        assert!(
            !format!("{result:?}").contains("C:"),
            "the absolute config path must not be captured"
        );
    }

    #[test]
    fn an_overridden_write_is_visible_rather_than_reported_as_plain_success() {
        let result: ConfigWriteResult = parse(
            r#"{"status":"ok","overriddenMetadata":{"type":"legacyManagedConfigTomlFromMdm"}}"#,
        )
        .expect("payload parses");

        assert!(result.overridden_metadata.is_some());
    }

    #[test]
    fn a_write_error_body_yields_the_server_defined_code() {
        // The exact body a live server returned for a stale `expectedVersion`.
        let data: ConfigWriteErrorData =
            parse(r#"{"config_write_error_code":"configVersionConflict"}"#)
                .expect("payload parses");

        assert_eq!(
            data.config_write_error_code.as_deref(),
            Some("configVersionConflict")
        );
    }
}
