//! The wire shapes `codex app-server` returns, and their conversion to domain types.
//!
//! Unknown fields are ignored so a new server field cannot break a read; missing required fields
//! are an error, never a default (an absent `usedPercent` does not mean zero).

use serde::Deserialize;

use crate::accounts::AccountIdentity;

/// `initialize` result. It carries no protocol version or capability list, so the version is
/// read from the user agent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeResult {
    pub user_agent: String,
}

/// `account/read` result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountReadResult {
    /// `null` means nobody is signed in.
    pub account: Option<AccountDto>,
    // `requiresOpenaiAuth` is not modelled: it stays `true` after a successful login.
}

/// The tagged union the server uses for an account.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum AccountDto {
    ApiKey,
    // `rename_all` on the enum renames variants only; without this `planType` silently
    // deserialises to `None`.
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
                // The server's `"unknown"` means no information, so it becomes `None`.
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
    /// The OAuth URL the user opens. Never logged: it carries PKCE parameters.
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
/// A user cancellation and a genuine failure both arrive as `success: false`; the caller tells
/// them apart by remembering that it asked to cancel.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginCompletedParams {
    pub login_id: String,
    pub success: bool,
}

/// `account/rateLimits/read` result.
///
/// `rateLimits` is the single-bucket view every version returns; newer servers add the
/// multi-bucket `rateLimitsByLimitId` beside it. Both are read; neither is required.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimitsResult {
    pub rate_limits: RateLimitsDto,
    #[serde(default)]
    pub rate_limits_by_limit_id: Option<std::collections::BTreeMap<String, RateLimitsDto>>,
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
    #[serde(default)]
    pub credits: Option<CreditsDto>,
}

/// The account's credit balance, returned beside the windows.
///
/// `balance` is a string on the wire and is carried through uninterpreted.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreditsDto {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(default)]
    pub balance: Option<String>,
}

/// One rate limit window exactly as returned.
///
/// Only `usedPercent` is required. A `null` `windowDurationMins` means the window type is
/// unknown; it must not be inferred from whether the window arrived as `primary` or `secondary`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowDto {
    /// A float so a fractional value is carried through rather than rejected; rounding is the
    /// display layer's decision.
    pub used_percent: f64,
    #[serde(default)]
    pub window_duration_mins: Option<i64>,
    #[serde(default)]
    pub resets_at: Option<i64>,
}

/// Quota data as the server gave it, with nothing classified or filled in; classifying windows
/// belongs to `quota`.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRateLimits {
    pub primary: Option<RawWindow>,
    pub secondary: Option<RawWindow>,
    pub plan_type: Option<String>,
    /// `None` when the server did not report credits at all.
    pub credits: Option<RawCredits>,
    /// The per-limit buckets, keyed by the server's `limit_id`. Empty on a server that does
    /// not send them; that is "not reported", not "no limits".
    pub by_limit_id: std::collections::BTreeMap<String, RawLimitBucket>,
}

/// One bucket of `rateLimitsByLimitId`, in the same raw form as the single view.
#[derive(Debug, Clone, PartialEq)]
pub struct RawLimitBucket {
    pub primary: Option<RawWindow>,
    pub secondary: Option<RawWindow>,
    pub plan_type: Option<String>,
    pub credits: Option<RawCredits>,
}

/// Credits exactly as reported; parsed, not displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    /// The server's own string; `None` when it sent `null`.
    pub balance: Option<String>,
}

impl From<CreditsDto> for RawCredits {
    fn from(dto: CreditsDto) -> Self {
        Self {
            has_credits: dto.has_credits,
            unlimited: dto.unlimited,
            balance: dto.balance,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawWindow {
    /// Exactly what the server reported; `quota::normalize` owns clamping and rounding.
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

impl From<RateLimitsDto> for RawLimitBucket {
    fn from(dto: RateLimitsDto) -> Self {
        Self {
            primary: dto.primary.map(RawWindow::from),
            secondary: dto.secondary.map(RawWindow::from),
            plan_type: dto.plan_type.filter(|plan| plan != "unknown"),
            credits: dto.credits.map(RawCredits::from),
        }
    }
}

impl From<RateLimitsResult> for RawRateLimits {
    fn from(result: RateLimitsResult) -> Self {
        let single = RawLimitBucket::from(result.rate_limits);
        Self {
            primary: single.primary,
            secondary: single.secondary,
            plan_type: single.plan_type,
            credits: single.credits,
            by_limit_id: result
                .rate_limits_by_limit_id
                .unwrap_or_default()
                .into_iter()
                .map(|(limit_id, bucket)| (limit_id, RawLimitBucket::from(bucket)))
                .collect(),
        }
    }
}

/// `configRequirements/read` result.
///
/// `requirements: null` means no organisation-enforced configuration. The contents are not
/// modelled: Toglet only needs to know whether any exist.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigRequirementsResult {
    #[serde(default)]
    pub requirements: Option<serde_json::Value>,
}

/// `config/read` result.
///
/// Only the key Toglet manages is modelled. The real response carries the merged configuration,
/// including provider URLs, API-key variable names and every trusted project path; serde skips
/// what is not asked for, so none of it can reach a log or an error.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigReadResult {
    #[serde(default)]
    pub config: ConfigValues,
    /// Per-key provenance: which layer supplied the value, and that layer's version token.
    #[serde(default)]
    pub origins: std::collections::BTreeMap<String, ConfigOrigin>,
}

/// Configuration values, which are snake_case on the wire unlike the rest of the protocol.
///
/// No `rename_all` on purpose: a camelCase rename makes the key silently deserialise to `None`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConfigValues {
    #[serde(default)]
    pub cli_auth_credentials_store: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigOrigin {
    pub name: ConfigLayer,
    /// Opaque token for the layer's current content. It is not a SHA-256 of the file, so it is
    /// passed back verbatim and never recomputed.
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
/// Any other layer, including types that do not exist yet, stops the write, since the value may
/// be enforced by an organisation.
pub(crate) const USER_CONFIG_LAYER: &str = "user";

/// `config/value/write` result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigWriteResult {
    /// The layer's new version token, to be presented as `expectedVersion` on the next write.
    #[serde(default)]
    pub version: Option<String>,
    /// Non-null when a higher-priority layer overrides what was just written, so it has no effect.
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
    pub overridden: bool,
}

/// A server-defined `config/value/write` failure, carried in the JSON-RPC error's `data`.
#[derive(Debug, Deserialize)]
pub(crate) struct ConfigWriteErrorData {
    #[serde(default)]
    pub config_write_error_code: Option<String>,
}

/// The Codex version from the user agent (`<name>/0.98.0 (...)`).
///
/// The format is undocumented, so an unparseable version yields `None` rather than a guess.
/// Nothing gates on it.
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

        let limits = RawRateLimits::from(result);
        assert_eq!(
            limits.primary,
            Some(RawWindow {
                used_percent: 2.0,
                window_duration_mins: Some(300),
                resets_at: Some(1_788_164_992),
            })
        );
        assert_eq!(limits.plan_type.as_deref(), Some("plus"));
        assert_eq!(
            limits.credits,
            Some(RawCredits {
                has_credits: false,
                unlimited: false,
                balance: Some("0".to_owned()),
            })
        );
        assert!(limits.by_limit_id.is_empty());
    }

    #[test]
    fn the_multi_bucket_view_is_read_beside_the_single_view() {
        // The shape recorded from a real 0.153.4 server.
        let result: RateLimitsResult = parse(
            r#"{"rateLimits":{"primary":{"usedPercent":61,"windowDurationMins":300,"resetsAt":1789187993},
                "secondary":{"usedPercent":62,"windowDurationMins":10080,"resetsAt":1789448843},
                "credits":{"hasCredits":false,"unlimited":false,"balance":"0"},"planType":"plus"},
                "rateLimitsByLimitId":{"codex":{"planType":"plus",
                "primary":{"usedPercent":61,"windowDurationMins":300,"resetsAt":1789187993},
                "secondary":{"usedPercent":62,"windowDurationMins":10080,"resetsAt":1789448843}}}}"#,
        )
        .expect("the recorded payload parses");

        let limits = RawRateLimits::from(result);

        let codex = limits
            .by_limit_id
            .get("codex")
            .expect("the codex bucket is present");
        assert_eq!(codex.primary, limits.primary);
        assert_eq!(codex.secondary, limits.secondary);
        assert_eq!(codex.plan_type.as_deref(), Some("plus"));
        assert_eq!(
            codex.credits, None,
            "the bucket carried no credits; none are invented"
        );
    }

    #[test]
    fn credits_without_a_balance_are_still_credits() {
        let result: RateLimitsResult = parse(
            r#"{"rateLimits":{"credits":{"hasCredits":true,"unlimited":true,"balance":null}}}"#,
        )
        .expect("payload parses");

        let credits = RawRateLimits::from(result)
            .credits
            .expect("credits are present");
        assert!(credits.unlimited);
        assert_eq!(credits.balance, None);
    }

    #[test]
    fn credits_missing_a_required_flag_are_refused() {
        let parsed: Result<RateLimitsResult, _> =
            parse(r#"{"rateLimits":{"credits":{"balance":"5"}}}"#);
        assert!(
            parsed.is_err(),
            "hasCredits cannot be guessed from a balance"
        );
    }

    #[test]
    fn an_unknown_field_is_ignored_rather_than_rejected() {
        let result: RateLimitsResult = parse(
            r#"{"rateLimits":{"primary":{"usedPercent":5,"somethingNew":42},"anotherNewThing":true}}"#,
        )
        .expect("a payload with new fields still parses");

        let limits = RawRateLimits::from(result);
        assert!(
            (limits.primary.expect("primary is present").used_percent - 5.0).abs() < f64::EPSILON
        );
    }

    #[test]
    fn a_missing_window_stays_missing_and_is_never_zero() {
        let result: RateLimitsResult =
            parse(r#"{"rateLimits":{"primary":{"usedPercent":7}}}"#).expect("payload parses");

        let limits = RawRateLimits::from(result);
        // The weekly window was not returned. That is not "0% used".
        assert_eq!(limits.secondary, None);
        let primary = limits.primary.expect("primary is present");
        assert!((primary.used_percent - 7.0).abs() < f64::EPSILON);
        // Nor is an absent duration a five-hour window.
        assert_eq!(primary.window_duration_mins, None);
        assert_eq!(primary.resets_at, None);
        // And absent credits are absent, not "no credits".
        assert_eq!(limits.credits, None);
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
        // A server with no enforced configuration returns this.
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
