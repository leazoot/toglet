//! JSON-RPC 2.0 over the NDJSON line transport, and the methods Toglet calls.
//!
//! `initialize` returns no protocol version, so compatibility is judged by behaviour: the version
//! is parsed for diagnostics only, and a runtime is incompatible when a needed method fails at the
//! protocol level. Every method used is on the stable v2 surface, so `experimentalApi` is `false`.

use std::path::Path;
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
use super::thread::{
    ModelInfo, ModelListResult, ServerEvent, TextInput, ThreadEnvelope, ThreadListParams,
    ThreadListResult, ThreadPage, ThreadReadParams, ThreadResumeParams, ThreadStatusChangedParams,
    ThreadSummary, TurnCompletedParams, TurnInterruptParams, TurnInterruptResult, TurnRecord,
    TurnStartParams,
};
use crate::accounts::AccountIdentity;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Deadline for calls the server answers locally, which normally take well under a second.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Deadline for calls that go out to the network; a rate limit read can take a few seconds.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for calls that read session files from disk; a large session directory on a slow
/// disk gets the network budget.
const DISK_TIMEOUT: Duration = NETWORK_TIMEOUT;

/// The most threads one `thread/list` asks for; a full page is reported as truncation.
const THREAD_PAGE_LIMIT: u32 = 100;

/// Newest-updated first, so the session the user last worked in is at the top.
const THREAD_SORT_KEY: &str = "updated_at";

/// JSON-RPC reserved error codes. Anything in this band is a protocol-level complaint - an
/// unknown method, a rejected request shape - which means this runtime cannot serve Toglet.
const RESERVED_ERROR_RANGE: std::ops::RangeInclusive<i64> = -32768..=-32000;

/// JSON-RPC "internal error": the server understood the request and failed to carry it out.
/// Inside the reserved range but not a statement about compatibility.
const INTERNAL_ERROR: i64 = -32603;

/// JSON-RPC "invalid request", which the server also returns, without `data`, for a thread it
/// cannot serve.
const INVALID_REQUEST: i64 = -32600;

/// The methods that act on one named thread. An invalid-request answer from one of these
/// means the thread, not the protocol, is the problem.
fn is_thread_method(method: &str) -> bool {
    matches!(
        method,
        "thread/read" | "thread/resume" | "turn/start" | "turn/interrupt"
    )
}

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
    /// Notifications and server-initiated requests seen while waiting for a response, in arrival
    /// order. They must be kept: `account/login/completed` or `turn/completed` can arrive while
    /// another request is in flight.
    pending: Vec<Frame>,
}

impl AppServerSession {
    /// Performs the `initialize` / `initialized` handshake.
    pub fn open(client: AppServerClient) -> Result<Self> {
        let mut session = Self {
            client,
            next_id: 1,
            runtime_version: None,
            pending: Vec::new(),
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

    /// The subprocess id, for the client probe's exclusion list.
    pub fn pid(&self) -> u32 {
        self.client.pid()
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
        Ok(RawRateLimits::from(result))
    }

    /// The threads the server can see, newest-updated first, optionally only those whose working
    /// directory is exactly `cwd`.
    ///
    /// A server silently hides sessions written by a newer Codex, so an empty page does not prove
    /// that no session exists.
    pub fn list_threads(&mut self, cwd: Option<&Path>) -> Result<ThreadPage> {
        let params = ThreadListParams {
            cwd,
            limit: THREAD_PAGE_LIMIT,
            sort_key: THREAD_SORT_KEY,
        };
        let result: ThreadListResult =
            self.call("thread/list", self.params(params)?, DISK_TIMEOUT)?;
        Ok(ThreadPage {
            threads: result.data.into_iter().map(ThreadSummary::from).collect(),
            truncated: result.next_cursor.is_some(),
        })
    }

    /// One thread with its turns, read from the session file without loading it.
    pub fn read_thread(&mut self, thread_id: &str) -> Result<ThreadSummary> {
        let params = ThreadReadParams {
            thread_id,
            include_turns: true,
        };
        let result: ThreadEnvelope =
            self.call("thread/read", self.params(params)?, LOCAL_TIMEOUT)?;
        Ok(ThreadSummary::from(result.thread))
    }

    /// Loads a thread into this server so a turn can be started on it.
    ///
    /// No overrides are sent, so a turn runs under the session's existing approval and sandbox
    /// policy; resuming does not change the session file.
    pub fn resume_thread(&mut self, thread_id: &str) -> Result<ThreadSummary> {
        let params = ThreadResumeParams { thread_id };
        let result: ThreadEnvelope =
            self.call("thread/resume", self.params(params)?, DISK_TIMEOUT)?;
        Ok(ThreadSummary::from(result.thread))
    }

    /// Starts one turn with `instruction` as the user message.
    ///
    /// Returns once the turn is accepted; the outcome arrives as a `turn/completed` event from
    /// [`Self::next_event`]. Callers own the once-per-exhaustion rule.
    pub fn start_turn(&mut self, thread_id: &str, instruction: &str) -> Result<TurnRecord> {
        let params = TurnStartParams {
            thread_id,
            input: [TextInput::new(instruction)],
        };
        let result: TurnStartedResult =
            self.call("turn/start", self.params(params)?, LOCAL_TIMEOUT)?;
        Ok(TurnRecord::from(result.turn))
    }

    /// Asks the server to stop a running turn. Returns once the request is accepted; the turn
    /// then reports `interrupted` through `turn/completed`.
    pub fn interrupt_turn(&mut self, thread_id: &str, turn_id: &str) -> Result<()> {
        let params = TurnInterruptParams { thread_id, turn_id };
        let TurnInterruptResult {} =
            self.call("turn/interrupt", self.params(params)?, LOCAL_TIMEOUT)?;
        Ok(())
    }

    /// The model catalogue this server offers; the same for every account, so display only.
    pub fn list_models(&mut self) -> Result<Vec<ModelInfo>> {
        let result: ModelListResult = self.call("model/list", json!({}), NETWORK_TIMEOUT)?;
        Ok(result.data.into_iter().map(ModelInfo::from).collect())
    }

    /// The next thing the server says on its own, buffered events first.
    ///
    /// Reaching the deadline is reported as the server being unresponsive, which callers must
    /// expect while a turn is still running.
    pub fn next_event(&mut self, timeout: Duration) -> Result<ServerEvent> {
        let phase = self.client.home().phase();
        if !self.pending.is_empty() {
            return server_event(self.pending.remove(0), phase);
        }

        // Nothing is in flight, so `server_event` rightly refuses a result or error frame.
        let line = self.client.recv_line(timeout)?;
        server_event(parse_frame(&line, phase)?, phase)
    }

    /// Serialises typed parameters. The types cannot fail to serialise; if one ever does it is
    /// a bug in this module, reported as such rather than as a server problem.
    fn params<T: serde::Serialize>(&self, params: T) -> Result<Value> {
        serde_json::to_value(params).map_err(|error| {
            TogletError::new(
                ErrorCode::Internal,
                self.client.home().phase(),
                false,
                UserAction::Retry,
            )
            .with_detail(&error.to_string())
        })
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

    /// Waits for the `account/login/completed` notification and returns whether it reported
    /// success; a cancellation also reads as `false`.
    pub fn await_login_completion(&mut self, login_id: &str, timeout: Duration) -> Result<bool> {
        let phase = self.client.home().phase();
        let deadline = std::time::Instant::now() + timeout;

        // Anything buffered while an earlier request was in flight is checked first.
        let buffered = std::mem::take(&mut self.pending);
        for frame in buffered {
            if let Frame::Notification { method, params } = frame
                && let Some(success) = login_result(&method, params, login_id, phase)?
            {
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

    /// Whether this session runs against the user's real Codex home. Configuration changes must
    /// check this: writing to a throwaway home would succeed and change nothing.
    pub fn home_is_default(&self) -> bool {
        self.client.home().is_default()
    }

    /// Whether an organisation-enforced configuration is present; anything non-null stops Toglet.
    pub fn organisation_requirements_present(&mut self) -> Result<bool> {
        let result: ConfigRequirementsResult =
            self.call("configRequirements/read", json!({}), LOCAL_TIMEOUT)?;
        Ok(result.requirements.is_some())
    }

    /// Reads the credential-store setting and the layer it came from.
    ///
    /// The returned version token guards the following write against concurrent edits.
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
    /// With `expected_version` from [`Self::read_credential_store_setting`], the server itself
    /// refuses the write if the file changed since, so there is no check-then-write window.
    /// `upsert` touches only this key and preserves comments.
    pub fn write_credential_store_setting(
        &mut self,
        value: &str,
        expected_version: Option<&str>,
    ) -> Result<ConfigWriteOutcome> {
        self.write_credential_store(json!(value), expected_version)
    }

    /// Removes the credential-store key, so Codex falls back to its previous behaviour.
    ///
    /// The protocol has no delete method, so this writes `null`: it removes only that line, keeps
    /// comments and layout, and succeeds without change when the key is already absent.
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

    /// Reads frames until the response with `id` arrives, buffering notifications and server
    /// requests. The deadline covers the whole wait, so a stream of notifications cannot extend it.
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
                frame @ (Frame::Notification { .. } | Frame::ServerRequest { .. }) => {
                    self.pending.push(frame);
                }
            }
        }
    }
}

/// `turn/start` result.
#[derive(Debug, serde::Deserialize)]
struct TurnStartedResult {
    turn: super::thread::TurnDto,
}

/// Gives a buffered or freshly read frame its meaning as an event.
fn server_event(frame: Frame, phase: Phase) -> Result<ServerEvent> {
    match frame {
        Frame::Notification { method, params } => match method.as_str() {
            "turn/completed" => {
                let completed: TurnCompletedParams = read_params(params, &method, phase)?;
                Ok(ServerEvent::TurnCompleted {
                    thread_id: completed.thread_id,
                    turn: TurnRecord::from(completed.turn),
                })
            }
            "thread/status/changed" => {
                let changed: ThreadStatusChangedParams = read_params(params, &method, phase)?;
                Ok(ServerEvent::ThreadStatusChanged {
                    thread_id: changed.thread_id,
                    status: changed.status.into(),
                })
            }
            _ => Ok(ServerEvent::Other { method }),
        },
        Frame::ServerRequest { method, .. } => Ok(ServerEvent::ServerRequest { method }),
        Frame::Result { .. } | Frame::Error { .. } => Err(incompatible(
            phase,
            "the app server answered a request that was never sent",
        )),
    }
}

fn read_params<T: DeserializeOwned>(params: Value, method: &str, phase: Phase) -> Result<T> {
    serde_json::from_value(params).map_err(|error| {
        incompatible(
            phase,
            "a notification from the app server could not be read",
        )
        .with_detail(&format!("{method}: {error}"))
    })
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
        /// The server's error payload. `config/value/write` reports why it refused here, using
        /// `-32600` for failures unrelated to protocol compatibility.
        data: Option<Value>,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A request from the server (an approval, an elicitation) that blocks until answered. Kept
    /// apart from notifications so it is never mistaken for one the session may ignore.
    ServerRequest {
        #[allow(
            dead_code,
            reason = "the id is what makes it a request; nothing here answers one"
        )]
        id: u64,
        method: String,
    },
}

/// Splits one NDJSON line into the shapes JSON-RPC allows.
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
        // A method with an id is the server asking, not telling. Its params are not kept:
        // an approval request quotes the command or file the model wants to touch.
        if let Some(id) = id {
            return Ok(Frame::ServerRequest {
                id,
                method: method.to_owned(),
            });
        }
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

    // A configuration Codex cannot parse: the server keeps running on defaults and answers every
    // config method with `-32603` and no `data`. Decided from code and method, never the message,
    // which carries the configuration file's absolute path.
    if code == INTERNAL_ERROR && is_config_method(method) {
        return TogletError::new(
            ErrorCode::ConfigSyntaxError,
            phase,
            false,
            UserAction::FixConfigManually,
        );
    }

    // A thread the server cannot serve. Decided from the code and the method, never from the
    // message, which quotes the thread id. A method this server does not have at all is still
    // `-32601`, so an old runtime is not mistaken for a missing session.
    if code == INVALID_REQUEST && is_thread_method(method) {
        return TogletError::new(
            ErrorCode::ThreadUnavailable,
            phase,
            false,
            UserAction::RebindSession,
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
/// Returns `None` only when the payload is not a config write failure. Any
/// `config_write_error_code` proves the server understood and declined, so an unknown code
/// becomes `Internal`, not incompatible. Codes with unobserved triggers are left unmapped.
fn config_write_error(phase: Phase, data: Value) -> Option<TogletError> {
    let parsed: ConfigWriteErrorData = serde_json::from_value(data).ok()?;
    let code = parsed.config_write_error_code?;

    let error = match code.as_str() {
        // A stale `expectedVersion`: nothing is written.
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
    fn a_server_request_is_told_apart_from_a_notification() {
        // The shape a real server used when a turn needed approval.
        let parsed = frame(
            r#"{"jsonrpc":"2.0","id":7,"method":"item/commandExecution/requestApproval",
                "params":{"command":"rm -rf /private/thing"}}"#,
        )
        .expect("a server request parses");

        let Frame::ServerRequest { id, method } = &parsed else {
            panic!("expected a server request, got {parsed:?}");
        };
        assert_eq!(*id, 7);
        assert_eq!(method, "item/commandExecution/requestApproval");
        assert!(
            !format!("{parsed:?}").contains("rm -rf"),
            "the request's params must not be kept"
        );
    }

    #[test]
    fn a_completed_turn_becomes_a_typed_event() {
        let parsed = frame(
            r#"{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"th",
                "turn":{"id":"t2","status":"failed","items":[],
                "error":{"message":"limit","codexErrorInfo":"usageLimitExceeded"}}}}"#,
        )
        .expect("a notification parses");

        let event = server_event(parsed, Phase::ReadQuota).expect("the event is readable");

        let ServerEvent::TurnCompleted { thread_id, turn } = event else {
            panic!("expected a completed turn");
        };
        assert_eq!(thread_id, "th");
        assert_eq!(turn.status, super::super::thread::TurnStatus::Failed);
        assert_eq!(
            turn.error,
            Some(super::super::thread::TurnErrorKind::UsageLimitExceeded)
        );
    }

    #[test]
    fn a_status_change_carries_its_flags_and_other_notifications_are_named() {
        let changed = frame(
            r#"{"jsonrpc":"2.0","method":"thread/status/changed","params":{"threadId":"th",
                "status":{"type":"active","activeFlags":["waitingOnUserInput"]}}}"#,
        )
        .expect("parses");
        let other = frame(r#"{"jsonrpc":"2.0","method":"thread/goal/cleared","params":{}}"#)
            .expect("parses");
        let request = frame(r#"{"jsonrpc":"2.0","id":1,"method":"item/tool/requestUserInput"}"#)
            .expect("parses");

        let changed = server_event(changed, Phase::ReadQuota).expect("readable");
        assert!(matches!(
            &changed,
            ServerEvent::ThreadStatusChanged { thread_id, status }
                if thread_id == "th" && status.is_waiting_on_human()
        ));
        assert_eq!(
            server_event(other, Phase::ReadQuota).expect("readable"),
            ServerEvent::Other {
                method: "thread/goal/cleared".to_owned()
            }
        );
        assert_eq!(
            server_event(request, Phase::ReadQuota).expect("readable"),
            ServerEvent::ServerRequest {
                method: "item/tool/requestUserInput".to_owned()
            }
        );
    }

    #[test]
    fn a_turn_notification_missing_its_thread_is_incompatible_not_a_guess() {
        let parsed = frame(
            r#"{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"t","status":"completed","items":[]}}}"#,
        )
        .expect("parses");

        let error = server_event(parsed, Phase::ReadQuota).expect_err("the thread id is required");

        assert_eq!(error.code(), ErrorCode::RuntimeIncompatible);
    }

    /// Starting a turn is the one capability that could make refreshing or switching act on the
    /// user's session; the language cannot forbid the call, so the source is scanned.
    #[test]
    fn only_the_continuation_path_can_resume_a_thread_or_start_a_turn() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();

        for module in ["quota", "switching"] {
            for entry in std::fs::read_dir(root.join(module)).expect("the module is readable") {
                let path = entry.expect("entry is readable").path();
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("source is readable");
                let production = text.split("#[cfg(test)]").next().unwrap_or_default();
                for forbidden in [
                    "turn/start",
                    "thread/resume",
                    "start_turn(",
                    "resume_thread(",
                ] {
                    if production.contains(forbidden) {
                        offenders.push(format!("{module}/{} uses {forbidden}", path.display()));
                    }
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "neither refreshing nor switching may act on a session: {offenders:?}"
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
    fn an_empty_isolated_home_lists_no_threads_and_refuses_an_unknown_one() {
        let mut session = open_session();

        // Proves the real server accepts the parameter shapes: a rejected shape comes back as
        // a protocol-level error, not as an empty page.
        let page = session
            .list_threads(None)
            .expect("thread/list succeeds on an empty home");
        assert!(page.threads.is_empty());
        assert!(!page.truncated);

        let filtered = session
            .list_threads(Some(session.home_path().as_path()))
            .expect("a cwd filter is accepted");
        assert!(filtered.threads.is_empty());

        let error = session
            .read_thread("00000000-0000-7000-8000-000000000000")
            .expect_err("a thread that does not exist is an error, not a fabricated thread");
        assert_eq!(
            error.code(),
            ErrorCode::ThreadUnavailable,
            "the server understood the request; the thread is simply not there"
        );
        assert!(!error.retryable());
        session.close().expect("the app server exits cleanly");
    }

    #[test]
    fn a_missing_thread_is_not_reported_as_an_incompatible_runtime() {
        // The server's answers for an unknown and a malformed thread id.
        for method in [
            "thread/read",
            "thread/resume",
            "turn/start",
            "turn/interrupt",
        ] {
            let error = rpc_error(Phase::ReadQuota, method, -32600, None);
            assert_eq!(error.code(), ErrorCode::ThreadUnavailable, "{method}");
            assert!(!error.retryable(), "{method}");
        }
        // But a server without the method at all is still an incompatible runtime ...
        assert_eq!(
            rpc_error(Phase::ReadQuota, "thread/resume", -32601, None).code(),
            ErrorCode::RuntimeIncompatible
        );
        // ... and the same code on a method that does not name a thread keeps its meaning.
        assert_eq!(
            rpc_error(Phase::ReadQuota, "thread/list", -32600, None).code(),
            ErrorCode::RuntimeIncompatible
        );
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
