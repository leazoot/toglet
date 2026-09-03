//! JSON-RPC 2.0 over the NDJSON line transport, and the methods Toglet calls.
//!
//! Compatibility is decided by behaviour, not by a version number. `initialize` returns no
//! protocol version and no capability list - only a user agent - so a numeric floor would have
//! to be invented, and inventing one would reject working builds that were simply never tested.
//! Instead the version is parsed for diagnostics and incompatibility is concluded when a method
//! Toglet needs actually fails at the protocol level. Every `account/*` method Toglet uses is
//! on the stable surface, so `experimentalApi` stays `false`.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::client::AppServerClient;
use super::dto::{
    AccountReadResult, CREDENTIAL_STORE_KEY, ConfigOrigin, ConfigReadResult,
    ConfigRequirementsResult, ConfigWriteErrorData, ConfigWriteOutcome, ConfigWriteResult,
    CredentialStoreSetting, InitializeResult, LoginCancelResult, LoginCompletedParams,
    LoginStartResult, RateLimitsResult, RawRateLimits, runtime_version,
};
use crate::accounts::AccountIdentity;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Deadline for calls the server answers locally. `account/read` was measured at 1 ms and cold
/// start plus handshake at 384 ms; this leaves room for a loaded machine without ever letting a
/// hung server block a refresh.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Deadline for calls that go out to the network. `rateLimits/read` was measured at 2496 ms,
/// with at least 10 s required.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);

/// JSON-RPC reserved error codes. Anything in this band is a protocol-level complaint - an
/// unknown method, a rejected request shape - which means this runtime cannot serve Toglet.
const RESERVED_ERROR_RANGE: std::ops::RangeInclusive<i64> = -32768..=-32000;

/// JSON-RPC "internal error": the server understood the request and failed to carry it out.
/// Inside the reserved range but not a statement about compatibility.
const INTERNAL_ERROR: i64 = -32603;

/// The methods that read or write Codex's configuration. An internal error from one of these
/// means the configuration itself could not be loaded.
fn is_config_method(method: &str) -> bool {
    matches!(
        method,
        "config/read" | "config/value/write" | "configRequirements/read"
    )
}

/// A handshaken app server, ready for method calls.
pub struct AppServerSession {
    client: AppServerClient,
    next_id: u64,
    runtime_version: Option<String>,
    /// Notifications seen while waiting for a response.
    ///
    /// They cannot be discarded: `account/login/completed` genuinely arrives while another
    /// request is in flight - a cancellation, for one - and throwing it away turns a cancelled
    /// sign-in into a timeout. Found by a test, not by review.
    pending_notifications: Vec<(String, Value)>,
}

impl AppServerSession {
    /// Performs the `initialize` / `initialized` handshake.
    pub fn open(client: AppServerClient) -> Result<Self> {
        let mut session = Self {
            client,
            next_id: 1,
            runtime_version: None,
            pending_notifications: Vec::new(),
        };

        let result: InitializeResult = session.call(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "toglet",
                    "title": "Toglet",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": { "experimentalApi": false },
            }),
            LOCAL_TIMEOUT,
        )?;
        session.runtime_version = runtime_version(&result.user_agent);

        // Required follow-up: the server expects this notification before it accepts work.
        session.notify("initialized", json!({}))?;
        Ok(session)
    }

    /// The Codex version, when it could be read from the user agent. Diagnostics only.
    pub fn runtime_version(&self) -> Option<&str> {
        self.runtime_version.as_deref()
    }

    /// The home this session runs against - throwaway for a quota read, the user's own when
    /// managing configuration.
    pub fn home_path(&self) -> std::path::PathBuf {
        self.client.home().path().to_path_buf()
    }

    /// Who the isolated home is signed in as, or `None` if nobody is.
    pub fn read_account(&mut self) -> Result<Option<AccountIdentity>> {
        let result: AccountReadResult = self.call(
            "account/read",
            json!({ "refreshToken": false }),
            LOCAL_TIMEOUT,
        )?;
        Ok(result.account.map(AccountIdentity::from))
    }

    /// The quota windows exactly as the server reported them.
    pub fn read_rate_limits(&mut self) -> Result<RawRateLimits> {
        let result: RateLimitsResult =
            self.call("account/rateLimits/read", Value::Null, NETWORK_TIMEOUT)?;
        Ok(RawRateLimits::from(result.rate_limits))
    }

    /// Starts a ChatGPT sign-in and returns the login id and the URL to open.
    pub fn login_start(&mut self) -> Result<(String, String)> {
        let result: LoginStartResult = self.call(
            "account/login/start",
            json!({ "type": "chatgpt" }),
            LOCAL_TIMEOUT,
        )?;
        Ok((result.login_id, result.auth_url))
    }

    /// Cancels a running sign-in. An unknown id is not an error: the login is not running,
    /// which is the state the caller asked for.
    pub fn login_cancel(&mut self, login_id: &str) -> Result<()> {
        let result: LoginCancelResult = self.call(
            "account/login/cancel",
            json!({ "loginId": login_id }),
            LOCAL_TIMEOUT,
        )?;
        match result.status.as_str() {
            "canceled" | "notFound" => Ok(()),
            other => Err(incompatible(
                self.client.home().phase(),
                "the app server answered a cancellation with an unknown status",
            )
            .with_detail(other)),
        }
    }

    /// Waits for the `account/login/completed` notification.
    ///
    /// Returns whether the server reported success. A `false` here means "did not complete" and
    /// nothing more - a cancellation looks exactly the same.
    pub fn await_login_completion(&mut self, login_id: &str, timeout: Duration) -> Result<bool> {
        let phase = self.client.home().phase();
        let deadline = std::time::Instant::now() + timeout;

        // Anything buffered while an earlier request was in flight is checked first.
        let buffered = std::mem::take(&mut self.pending_notifications);
        for (method, params) in buffered {
            if let Some(success) = login_result(&method, params, login_id, phase)? {
                return Ok(success);
            }
        }

        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self.client.recv_line(remaining)?;

            if let Frame::Notification { method, params } = parse_frame(&line, phase)? {
                if let Some(success) = login_result(&method, params, login_id, phase)? {
                    return Ok(success);
                }
            }
        }
    }

    /// The operation every error from this session is reported against.
    pub fn phase(&self) -> Phase {
        self.client.home().phase()
    }

    /// Whether this session runs against the user's real Codex home rather than a throwaway
    /// one. Anything that intends to change the user's own configuration must check this:
    /// writing to a throwaway home would succeed and change nothing.
    pub fn home_is_default(&self) -> bool {
        self.client.home().is_default()
    }

    /// Whether an organisation-enforced configuration is present.
    ///
    /// `requirements: null` means none (measured). The contents are not inspected: Toglet only
    /// needs to know whether to stop, and it stops on anything non-null.
    pub fn organisation_requirements_present(&mut self) -> Result<bool> {
        let result: ConfigRequirementsResult =
            self.call("configRequirements/read", json!({}), LOCAL_TIMEOUT)?;
        Ok(result.requirements.is_some())
    }

    /// Reads the credential-store setting and the layer it came from.
    ///
    /// The returned version token is what makes the following write safe against another tool
    /// editing the same file. `None` for the value means the key is not set anywhere;
    /// `None` for the layer means no layer claims it yet.
    pub fn read_credential_store_setting(&mut self) -> Result<CredentialStoreSetting> {
        let result: ConfigReadResult = self.call("config/read", json!({}), LOCAL_TIMEOUT)?;
        let origin = result.origins.get(CREDENTIAL_STORE_KEY);

        Ok(CredentialStoreSetting {
            value: result.config.cli_auth_credentials_store,
            written_by_user_layer: origin.map(ConfigOrigin::layer_type_is_user),
            version: origin.and_then(|origin| origin.version.clone()),
        })
    }

    /// Sets the credential store to file mode.
    ///
    /// `expected_version` is the token from [`Self::read_credential_store_setting`]. When it is
    /// supplied and the file has changed since, the server refuses the write and nothing is
    /// modified - the conflict is detected by the process that owns the file rather than by a
    /// re-read here, so there is no window between checking and writing.
    ///
    /// `upsert` rather than `replace`: the key is added or updated and the rest of the document
    /// is left alone. Comment preservation and idempotency were both measured.
    pub fn write_credential_store_setting(
        &mut self,
        value: &str,
        expected_version: Option<&str>,
    ) -> Result<ConfigWriteOutcome> {
        self.write_credential_store(json!(value), expected_version)
    }

    /// Removes the credential-store key, so Codex falls back to whatever it did before Toglet
    /// set it.
    ///
    /// The protocol has no delete method - the complete method list was read out of the runtime
    /// binary - so removal is expressed as a write of `null`. Measured against the real server:
    /// it deletes exactly that one line and leaves comments, blank lines and the surrounding
    /// tables untouched, and sending it for a key that is already absent succeeds without
    /// changing the file, which is what makes a restore repeatable.
    pub fn remove_credential_store_setting(
        &mut self,
        expected_version: Option<&str>,
    ) -> Result<ConfigWriteOutcome> {
        self.write_credential_store(Value::Null, expected_version)
    }

    fn write_credential_store(
        &mut self,
        value: Value,
        expected_version: Option<&str>,
    ) -> Result<ConfigWriteOutcome> {
        let mut params = json!({
            "keyPath": CREDENTIAL_STORE_KEY,
            "value": value,
            "mergeStrategy": "upsert",
        });
        if let Some(version) = expected_version {
            params["expectedVersion"] = json!(version);
        }

        let result: ConfigWriteResult = self.call("config/value/write", params, LOCAL_TIMEOUT)?;

        Ok(ConfigWriteOutcome {
            version: result.version,
            overridden: result.overridden_metadata.is_some(),
        })
    }

    /// Shuts the subprocess down and reports whether it exited cleanly.
    pub fn close(self) -> Result<()> {
        self.client.shutdown()
    }

    fn call<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<T> {
        let phase = self.client.home().phase();
        let id = self.next_id;
        self.next_id += 1;

        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.client.send_line(&request.to_string())?;

        let result = self.await_response(id, method, timeout, phase)?;
        serde_json::from_value(result).map_err(|error| {
            incompatible(phase, "the app server returned a result Toglet cannot read")
                .with_detail(&error.to_string())
        })
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.client.send_line(&notification.to_string())
    }

    /// Reads frames until the response with `id` arrives.
    ///
    /// The server interleaves notifications (`account/updated`, `account/rateLimits/updated`)
    /// with responses, so anything without a matching id is skipped rather than mistaken for
    /// the answer. The deadline covers the whole wait, not each individual frame, so a stream
    /// of notifications cannot extend it indefinitely.
    fn await_response(
        &mut self,
        id: u64,
        method: &str,
        timeout: Duration,
        phase: Phase,
    ) -> Result<Value> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self.client.recv_line(remaining)?;

            match parse_frame(&line, phase)? {
                Frame::Result { id: frame_id, .. } | Frame::Error { id: frame_id, .. }
                    if frame_id != Some(id) =>
                {
                    // A reply to something else. Toglet issues one request at a time, so this
                    // is a protocol violation rather than a race worth tolerating.
                    return Err(incompatible(
                        phase,
                        "the app server answered a request that was never sent",
                    ));
                }
                Frame::Result { result, .. } => return Ok(result),
                Frame::Error { code, data, .. } => {
                    return Err(rpc_error(phase, method, code, data));
                }
                Frame::Notification { method, params } => {
                    self.pending_notifications.push((method, params));
                }
            }
        }
    }
}

/// Reads a login completion notification, if that is what this is and it is for `login_id`.
fn login_result(method: &str, params: Value, login_id: &str, phase: Phase) -> Result<Option<bool>> {
    if method != "account/login/completed" {
        return Ok(None);
    }
    let completed: LoginCompletedParams = serde_json::from_value(params).map_err(|error| {
        incompatible(phase, "the login notification could not be read")
            .with_detail(&error.to_string())
    })?;
    // A notification for some other sign-in is not this one's answer.
    Ok((completed.login_id == login_id).then_some(completed.success))
}

#[derive(Debug)]
enum Frame {
    Result {
        id: Option<u64>,
        result: Value,
    },
    Error {
        id: Option<u64>,
        code: i64,
        /// The server's own error payload. Kept because `config/value/write` reports *why* it
        /// refused in here, using the reserved code `-32600` for failures that have nothing to
        /// do with protocol compatibility. Dropping it turned a routine "someone else edited
        /// the file" into "your Codex runtime is incompatible - update it".
        data: Option<Value>,
    },
    Notification {
        method: String,
        params: Value,
    },
}

/// Splits one NDJSON line into the three shapes JSON-RPC allows.
fn parse_frame(line: &str, phase: Phase) -> Result<Frame> {
    let frame: Value = serde_json::from_str(line).map_err(|error| {
        incompatible(phase, "the app server sent something that is not JSON")
            .with_detail(&error.to_string())
    })?;

    let id = frame.get("id").and_then(Value::as_u64);
    if let Some(error) = frame.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .ok_or_else(|| incompatible(phase, "the app server sent an error without a code"))?;
        return Ok(Frame::Error {
            id,
            code,
            data: error.get("data").cloned(),
        });
    }
    if let Some(result) = frame.get("result") {
        return Ok(Frame::Result {
            id,
            result: result.clone(),
        });
    }
    if let Some(method) = frame.get("method").and_then(Value::as_str) {
        return Ok(Frame::Notification {
            method: method.to_owned(),
            params: frame.get("params").cloned().unwrap_or(Value::Null),
        });
    }
    Err(incompatible(
        phase,
        "the app server sent a frame that is neither a result, an error nor a notification",
    ))
}

fn rpc_error(phase: Phase, method: &str, code: i64, data: Option<Value>) -> TogletError {
    // Checked before the reserved range, because the server reports config write failures with
    // `-32600` even though they are not protocol complaints.
    if let Some(config_error) = data.and_then(|data| config_write_error(phase, data)) {
        return config_error;
    }

    // A configuration Codex cannot parse. Measured: the server logs the parse position, keeps
    // running on defaults, and answers every config method with `-32603` and **no** `data`
    // field. Without this branch the user is told to update their runtime because of a typo in
    // their own file, and the file is never mentioned.
    //
    // The decision is made from the code and the method, never from the message: that message
    // carries the absolute path of the configuration file, which must not enter an error
    // Toglet stores or shows.
    if code == INTERNAL_ERROR && is_config_method(method) {
        return TogletError::new(
            ErrorCode::ConfigSyntaxError,
            phase,
            false,
            UserAction::FixConfigManually,
        );
    }

    // `-32603` is the server failing to process a request it understood perfectly well. That
    // is not evidence of an incompatible runtime, so it is not reported as one.
    if code == INTERNAL_ERROR {
        return TogletError::new(ErrorCode::Internal, phase, true, UserAction::Retry)
            .with_detail(&format!("json-rpc code {code}"));
    }

    if RESERVED_ERROR_RANGE.contains(&code) {
        incompatible(
            phase,
            "the app server rejected the request at the protocol level",
        )
        .with_detail(&format!("json-rpc code {code}"))
    } else {
        // A server-defined failure. Nothing here can tell what it means, so it is reported as
        // an internal failure with the code kept for diagnosis rather than guessed at.
        TogletError::new(ErrorCode::Internal, phase, true, UserAction::Retry)
            .with_detail(&format!("json-rpc code {code}"))
    }
}

/// Translates a `config/value/write` refusal into a Toglet error code.
///
/// Returns `None` only when the payload is not a config write failure at all. The presence of
/// `config_write_error_code` is itself proof that the server understood the request and
/// declined it, so even a code Toglet has never seen must not be reported as an incompatible
/// runtime - it becomes `Internal` with the server's own code kept for diagnosis.
///
/// Three of the six codes the runtime defines are mapped deliberately; the rest are left
/// unmapped because their trigger conditions have not been observed, and guessing at them
/// would put wrong advice in front of the user.
fn config_write_error(phase: Phase, data: Value) -> Option<TogletError> {
    let parsed: ConfigWriteErrorData = serde_json::from_value(data).ok()?;
    let code = parsed.config_write_error_code?;

    let error = match code.as_str() {
        // Reproduced against a live server with a stale `expectedVersion`: nothing is written.
        "configVersionConflict" => {
            TogletError::new(ErrorCode::ConfigConflict, phase, true, UserAction::Retry)
        }
        // The layer holding the key refuses writes, which is how an organisation's managed
        // configuration presents itself. Retrying cannot help.
        "configLayerReadonly" => TogletError::new(
            ErrorCode::ConfigLayerReadonly,
            phase,
            false,
            UserAction::FixConfigManually,
        ),
        // This runtime does not know the key Toglet manages, so it cannot serve Toglet.
        "configSchemaUnknownKey" => incompatible(
            phase,
            "this runtime does not recognise the credential store setting",
        ),
        "configValidationError" => TogletError::new(
            ErrorCode::ConfigSyntaxError,
            phase,
            false,
            UserAction::FixConfigManually,
        ),
        other => TogletError::new(ErrorCode::Internal, phase, true, UserAction::Retry)
            .with_detail(&format!("config write error {other}")),
    };
    Some(error)
}

fn incompatible(phase: Phase, detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::RuntimeIncompatible,
        phase,
        false,
        UserAction::UpdateRuntime,
    )
    .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_server::CodexBinary;
    use crate::codex_home::IsolatedHome;

    fn frame(line: &str) -> Result<Frame> {
        parse_frame(line, Phase::ReadQuota)
    }

    #[test]
    fn a_result_frame_carries_its_id_and_payload() {
        let parsed = frame(r#"{"jsonrpc":"2.0","id":3,"result":{"userAgent":"x/1.0.0"}}"#)
            .expect("a result frame parses");

        match parsed {
            Frame::Result { id, result } => {
                assert_eq!(id, Some(3));
                assert_eq!(result["userAgent"], "x/1.0.0");
            }
            _ => panic!("expected a result frame"),
        }
    }

    #[test]
    fn a_notification_is_not_mistaken_for_a_response() {
        let parsed = frame(r#"{"jsonrpc":"2.0","method":"account/updated","params":{}}"#)
            .expect("a notification parses");

        assert!(
            matches!(parsed, Frame::Notification { ref method, .. } if method == "account/updated")
        );
    }

    #[test]
    fn a_non_json_line_reports_incompatible_rather_than_panicking() {
        let error = frame("not json at all").expect_err("garbage is rejected");

        assert_eq!(error.code(), ErrorCode::RuntimeIncompatible);
        assert!(!error.retryable());
    }

    #[test]
    fn a_frame_that_is_none_of_the_three_shapes_is_rejected() {
        assert_eq!(
            frame(r#"{"jsonrpc":"2.0","id":1}"#)
                .expect_err("an empty frame is rejected")
                .code(),
            ErrorCode::RuntimeIncompatible
        );
    }

    #[test]
    fn a_protocol_level_error_maps_to_incompatible() {
        // The exact error observed when requesting before the handshake.
        let parsed = frame(r#"{"error":{"code":-32600,"message":"Not initialized"},"id":9}"#)
            .expect("an error frame parses");
        let Frame::Error { code, .. } = parsed else {
            panic!("expected an error frame");
        };

        assert_eq!(
            rpc_error(Phase::ReadQuota, "account/read", code, None).code(),
            ErrorCode::RuntimeIncompatible
        );
    }

    #[test]
    fn a_config_write_refusal_is_read_out_of_the_error_data() {
        // The server uses the reserved code `-32600` for these, so the code alone would send
        // every one of them to "your runtime is incompatible - update it".
        let parsed = frame(
            r#"{"error":{"code":-32600,"message":"Configuration was modified since last read.",
                "data":{"config_write_error_code":"configVersionConflict"}},"id":3}"#,
        )
        .expect("an error frame parses");
        let Frame::Error { code, data, .. } = parsed else {
            panic!("expected an error frame");
        };

        let error = rpc_error(Phase::Write, "config/value/write", code, data);

        assert_eq!(error.code(), ErrorCode::ConfigConflict);
        assert!(error.retryable());
    }

    #[test]
    fn a_read_only_layer_is_distinguished_from_a_conflict() {
        let data = Some(json!({ "config_write_error_code": "configLayerReadonly" }));

        let error = rpc_error(Phase::Write, "config/value/write", -32600, data);

        assert_eq!(error.code(), ErrorCode::ConfigLayerReadonly);
        assert!(
            !error.retryable(),
            "retrying cannot make a managed layer writable"
        );
    }

    #[test]
    fn an_unrecognised_config_refusal_is_not_called_a_runtime_problem() {
        let data = Some(json!({ "config_write_error_code": "somethingAddedLater" }));

        assert_eq!(
            rpc_error(Phase::Write, "config/value/write", -32600, data).code(),
            ErrorCode::Internal,
            "the server understood the request and declined it; that is not incompatibility"
        );
    }

    #[test]
    fn an_error_without_config_data_still_maps_by_its_code() {
        let data = Some(json!({ "somethingElse": true }));

        assert_eq!(
            rpc_error(Phase::ReadQuota, "account/read", -32600, data).code(),
            ErrorCode::RuntimeIncompatible
        );
    }

    #[test]
    fn a_server_defined_error_is_not_reported_as_incompatible() {
        assert_eq!(
            rpc_error(Phase::ReadQuota, "account/read", 42, None).code(),
            ErrorCode::Internal,
            "an unrecognised failure must not be dressed up as a version problem"
        );
    }

    /// Drives the real `codex app-server` against an empty isolated home. Needs Codex
    /// installed; needs no account and no network.
    fn open_session() -> AppServerSession {
        let binary = CodexBinary::resolve(Phase::ReadQuota)
            .expect("Codex must be installed to run the app server tests");
        let home = IsolatedHome::create(Phase::ReadQuota).expect("isolated home is created");
        let client = AppServerClient::start(&binary, home).expect("the app server starts");
        AppServerSession::open(client).expect("the handshake succeeds")
    }

    #[test]
    fn the_handshake_reports_a_runtime_version() {
        let session = open_session();

        let version = session
            .runtime_version()
            .expect("the user agent carried a version")
            .to_owned();
        assert!(
            version.starts_with(|c: char| c.is_ascii_digit()),
            "unexpected version shape"
        );
        session.close().expect("the app server exits cleanly");
    }

    #[test]
    fn an_empty_isolated_home_reports_nobody_signed_in() {
        let mut session = open_session();

        let account = session.read_account().expect("account/read succeeds");

        // Not an error and not a fabricated account: an empty home genuinely has no identity.
        assert_eq!(account, None);
        session.close().expect("the app server exits cleanly");
    }
}
