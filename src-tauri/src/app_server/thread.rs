//! Thread, turn, model and notification shapes of `codex app-server`, and their domain forms.
//!
//! Session content, on-disk paths and free-text error messages are not modelled, so they cannot
//! reach a log or an error. The exception is a one-line `preview` excerpt, shown to tell sessions
//! apart but kept out of `Debug`, logs, errors and the stored plan.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------------------------
// Request parameters
// ---------------------------------------------------------------------------------------------

/// `thread/list` parameters. `cwd` is an exact-match filter.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadListParams<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<&'a Path>,
    pub limit: u32,
    pub sort_key: &'static str,
}

/// `thread/read` parameters. Turns are always requested: the last turn's status and error are
/// the whole point of reading a thread.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadReadParams<'a> {
    pub thread_id: &'a str,
    pub include_turns: bool,
}

/// `thread/resume` parameters.
///
/// Deliberately has no `approvalPolicy`, `sandbox`, `cwd` or `model` overrides, so the session
/// keeps exactly the permissions it had; a test checks the serialised keys.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeParams<'a> {
    pub thread_id: &'a str,
}

/// `turn/start` parameters, with no override fields.
///
/// The instruction is user-authored and travels only here, as JSON on stdin: never on a command
/// line, in the environment, a log or an error.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnStartParams<'a> {
    pub thread_id: &'a str,
    pub input: [TextInput<'a>; 1],
}

/// The one `UserInput` variant Toglet sends.
#[derive(Debug, Serialize)]
pub(crate) struct TextInput<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: &'a str,
}

impl<'a> TextInput<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { kind: "text", text }
    }
}

/// `turn/interrupt` parameters.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnInterruptParams<'a> {
    pub thread_id: &'a str,
    pub turn_id: &'a str,
}

// ---------------------------------------------------------------------------------------------
// Wire results
// ---------------------------------------------------------------------------------------------

/// `thread/list` result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadListResult {
    pub data: Vec<ThreadDto>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// `thread/read` and `thread/resume` results both wrap the thread the same way.
///
/// The effective approval policy, sandbox and cwd in the resume response are not read, because
/// no overrides are sent.
#[derive(Debug, Deserialize)]
pub(crate) struct ThreadEnvelope {
    pub thread: ThreadDto,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadDto {
    pub id: String,
    /// Absolute path of the project folder, needed to bind a session. Stored in the plan and shown
    /// only as a folder name; never logged.
    pub cwd: PathBuf,
    /// Version of the Codex that wrote the session; an older server cannot list it.
    pub cli_version: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// The user-facing title, when the desktop app has generated one.
    #[serde(default)]
    pub name: Option<String>,
    /// The first user message; see the module note.
    #[serde(default)]
    pub preview: Option<Preview>,
    pub status: ThreadStatusDto,
    /// Populated only by `thread/read` with `includeTurns` and by `thread/resume`.
    #[serde(default)]
    pub turns: Vec<TurnDto>,
}

/// The first user message as the server sends it. Its `Debug` says nothing, so that a captured
/// DTO - the form that ends up in error details - cannot quote it.
#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct Preview(String);

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Preview(..)")
    }
}

/// The longest excerpt of a preview the interface is given, in characters.
const PREVIEW_EXCERPT_CHARS: usize = 60;

/// The longest excerpt of an agent message, in characters (BATCH-07, user 2026-09-16).
///
/// Longer than a preview because this one has to be read, not just recognised: the user answers
/// it from a phone. Still short enough that a whole code block cannot ride out on it.
const MESSAGE_EXCERPT_CHARS: usize = 300;

impl Preview {
    /// The message as one line, cut to [`PREVIEW_EXCERPT_CHARS`], or `None` when it is blank.
    fn excerpt(&self) -> Option<String> {
        crate::text::one_line(&self.0, PREVIEW_EXCERPT_CHARS)
    }
}

/// An agent message as the server sends it. Same discipline as [`Preview`]: its `Debug` says
/// nothing, so a captured DTO cannot quote the session.
#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct Message(String);

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Message(..)")
    }
}

impl Message {
    fn excerpt(&self) -> Option<String> {
        crate::text::one_line(&self.0, MESSAGE_EXCERPT_CHARS)
    }
}

/// One entry of a turn. **Only the agent's own message is modelled**: commands, file changes,
/// tool calls and reasoning are session content and must not be materialised, so every other
/// kind lands on `Other` and keeps nothing.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum TurnItemDto {
    AgentMessage {
        text: Message,
    },
    #[serde(other)]
    Other,
}

/// The tagged union the server uses for a thread's runtime status.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ThreadStatusDto {
    NotLoaded,
    Idle,
    SystemError,
    #[serde(rename_all = "camelCase")]
    Active {
        active_flags: Vec<ActiveFlag>,
    },
    /// A status this build has never heard of. Reported as unknown rather than refused, so a
    /// newer server does not make every thread unreadable.
    #[serde(other)]
    Unknown,
}

/// Why an active thread is not making progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActiveFlag {
    /// The turn is blocked on an approval request that only a person can answer.
    WaitingOnApproval,
    /// The model asked the user a question.
    WaitingOnUserInput,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnDto {
    pub id: String,
    pub status: TurnStatus,
    /// Only populated when `status` is `failed`.
    #[serde(default)]
    pub error: Option<TurnErrorDto>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
    /// Read only to find the agent's last message; see [`TurnItemDto`].
    #[serde(default)]
    pub items: Vec<TurnItemDto>,
}

/// A turn's final state, exactly the server's four values plus an escape hatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
    #[serde(other)]
    Unknown,
}

/// The one field of `TurnError` that is machine-readable.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnErrorDto {
    #[serde(default)]
    pub codex_error_info: Option<CodexErrorInfoDto>,
}

/// `CodexErrorInfo` is a bare string for most values and a single-key object for variants that
/// carry data (`{"httpConnectionFailed": {"httpStatusCode": 502}}`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum CodexErrorInfoDto {
    Named(String),
    Structured(BTreeMap<String, Value>),
}

/// `model/list` result.
#[derive(Debug, Deserialize)]
pub(crate) struct ModelListResult {
    pub data: Vec<ModelDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelDto {
    pub id: String,
    pub display_name: String,
    pub is_default: bool,
}

/// `turn/interrupt` result: an empty object.
#[derive(Debug, Deserialize)]
pub(crate) struct TurnInterruptResult {}

/// The `turn/completed` notification.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnCompletedParams {
    pub thread_id: String,
    pub turn: TurnDto,
}

/// The `thread/status/changed` notification.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadStatusChangedParams {
    pub thread_id: String,
    pub status: ThreadStatusDto,
}

// ---------------------------------------------------------------------------------------------
// Domain forms
// ---------------------------------------------------------------------------------------------

/// One page of `thread/list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPage {
    pub threads: Vec<ThreadSummary>,
    /// The server had more than fit in one page. Reported rather than silently dropped: a
    /// session that is not in the list must be explained, not assumed absent.
    pub truncated: bool,
}

/// A thread as the server described it, with only the fields Toglet needs.
#[derive(Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub id: String,
    /// Absolute project path. See [`ThreadDto::cwd`] for where it may and may not go.
    pub cwd: PathBuf,
    pub cli_version: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds.
    pub updated_at: i64,
    pub title: Option<String>,
    /// A one-line excerpt of the first user message; shown, never logged or stored.
    pub preview: Option<String>,
    pub status: ThreadStatus,
    /// Empty unless the thread came from `thread/read` or `thread/resume`.
    pub turns: Vec<TurnRecord>,
}

impl ThreadSummary {
    /// The most recent turn, when turns were loaded. `None` says "not loaded", not "no turns":
    /// a caller that needs the distinction must read the thread with turns.
    pub fn last_turn(&self) -> Option<&TurnRecord> {
        self.turns.last()
    }

    /// The last path component, which is what the interface shows instead of the path.
    pub fn folder_name(&self) -> Option<String> {
        self.cwd
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

/// Hand-written so that `{:?}` - the form that ends up in error details - shows the folder
/// name and never the absolute path.
impl std::fmt::Debug for ThreadSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadSummary")
            .field("id", &self.id)
            .field("folder", &self.folder_name())
            .field("cli_version", &self.cli_version)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("title", &self.title)
            .field("status", &self.status)
            .field("turns", &self.turns)
            .finish()
    }
}

/// A thread's runtime status as seen by the server that answered.
///
/// Only the answering process's own loaded threads are `Idle` or `Active`; a thread the desktop
/// app has open reads as `NotLoaded`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadStatus {
    NotLoaded,
    Idle,
    SystemError,
    Active(Vec<ActiveFlag>),
    Unknown,
}

impl ThreadStatus {
    /// Whether the thread is stuck on something only a person can do.
    pub fn is_waiting_on_human(&self) -> bool {
        matches!(self, Self::Active(flags) if flags.iter().any(|flag| {
            matches!(flag, ActiveFlag::WaitingOnApproval | ActiveFlag::WaitingOnUserInput)
        }))
    }
}

impl From<ThreadStatusDto> for ThreadStatus {
    fn from(dto: ThreadStatusDto) -> Self {
        match dto {
            ThreadStatusDto::NotLoaded => Self::NotLoaded,
            ThreadStatusDto::Idle => Self::Idle,
            ThreadStatusDto::SystemError => Self::SystemError,
            ThreadStatusDto::Active { active_flags } => Self::Active(active_flags),
            ThreadStatusDto::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub id: String,
    pub status: TurnStatus,
    /// Present only for a failed turn, and only when the server gave a machine-readable reason.
    /// `None` on a failed turn means the reason is unknown - not that there was none.
    pub error: Option<TurnErrorKind>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    /// A one-line excerpt of the agent's last message in this turn, at most
    /// [`MESSAGE_EXCERPT_CHARS`]. `None` when the turn has no agent message, when the items were
    /// not loaded, or when the message is blank - **never an empty string**, which would read as
    /// "the agent said nothing".
    pub agent_excerpt: Option<String>,
}

/// Hand-written for the same reason [`ThreadSummary`]'s is: `{:?}` ends up in error details, and
/// the excerpt is session content. Deriving `Debug` here would undo the redaction above.
impl std::fmt::Debug for TurnRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnRecord")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("error", &self.error)
            .field("started_at", &self.started_at)
            .field("completed_at", &self.completed_at)
            .field("agent_excerpt", &self.agent_excerpt.as_ref().map(|_| ".."))
            .finish()
    }
}

impl From<TurnDto> for TurnRecord {
    fn from(dto: TurnDto) -> Self {
        Self {
            id: dto.id,
            status: dto.status,
            error: dto
                .error
                .and_then(|error| error.codex_error_info)
                .map(TurnErrorKind::from),
            started_at: dto.started_at,
            completed_at: dto.completed_at,
            // The last one wins: a turn may say several things, and what the user is answering
            // is the last of them.
            agent_excerpt: dto
                .items
                .iter()
                .filter_map(|item| match item {
                    TurnItemDto::AgentMessage { text } => text.excerpt(),
                    TurnItemDto::Other => None,
                })
                .next_back(),
        }
    }
}

/// Why a turn failed, as a stable enumeration over the server's `CodexErrorInfo`.
///
/// Every server value lands on exactly one variant; values added later land on `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnErrorKind {
    /// The account's usage window is exhausted; the one reason automatic continuation reacts to.
    UsageLimitExceeded,
    /// A server-side rate limit. Whether it counts as exhaustion is decided by what the
    /// windows and the turn's own error say, not here.
    RateLimitExceeded,
    ContextWindowExceeded,
    SessionBudgetExceeded,
    ServerOverloaded,
    InternalServerError,
    /// The credentials were refused.
    Unauthorized,
    BadRequest,
    /// Connecting to or streaming from the model failed. Covers all four transport variants
    /// the server distinguishes; the HTTP status, when it forwarded one, is kept.
    Network {
        http_status: Option<u16>,
    },
    /// `turn/start` was submitted while a turn that cannot be steered was still running.
    ActiveTurnNotSteerable,
    /// Every other value, known or not.
    Other,
}

impl TurnErrorKind {
    /// The stable code, for a wait reason or a log line. Never the server's message.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsageLimitExceeded => "usage_limit_exceeded",
            Self::RateLimitExceeded => "rate_limit_exceeded",
            Self::ContextWindowExceeded => "context_window_exceeded",
            Self::SessionBudgetExceeded => "session_budget_exceeded",
            Self::ServerOverloaded => "server_overloaded",
            Self::InternalServerError => "internal_server_error",
            Self::Unauthorized => "unauthorized",
            Self::BadRequest => "bad_request",
            Self::Network { .. } => "network",
            Self::ActiveTurnNotSteerable => "active_turn_not_steerable",
            Self::Other => "other",
        }
    }

    /// The inverse of [`as_str`](Self::as_str). A network kind read back has no HTTP status:
    /// the code does not carry one.
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "usage_limit_exceeded" => Self::UsageLimitExceeded,
            "rate_limit_exceeded" => Self::RateLimitExceeded,
            "context_window_exceeded" => Self::ContextWindowExceeded,
            "session_budget_exceeded" => Self::SessionBudgetExceeded,
            "server_overloaded" => Self::ServerOverloaded,
            "internal_server_error" => Self::InternalServerError,
            "unauthorized" => Self::Unauthorized,
            "bad_request" => Self::BadRequest,
            "network" => Self::Network { http_status: None },
            "active_turn_not_steerable" => Self::ActiveTurnNotSteerable,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

impl From<CodexErrorInfoDto> for TurnErrorKind {
    fn from(dto: CodexErrorInfoDto) -> Self {
        match dto {
            CodexErrorInfoDto::Named(name) => match name.as_str() {
                "usageLimitExceeded" => Self::UsageLimitExceeded,
                "rateLimitExceeded" => Self::RateLimitExceeded,
                "contextWindowExceeded" => Self::ContextWindowExceeded,
                "sessionBudgetExceeded" => Self::SessionBudgetExceeded,
                "serverOverloaded" => Self::ServerOverloaded,
                "internalServerError" => Self::InternalServerError,
                "unauthorized" => Self::Unauthorized,
                "badRequest" => Self::BadRequest,
                _ => Self::Other,
            },
            CodexErrorInfoDto::Structured(fields) => {
                let Some((variant, body)) = fields.into_iter().next() else {
                    return Self::Other;
                };
                match variant.as_str() {
                    "httpConnectionFailed"
                    | "responseStreamConnectionFailed"
                    | "responseStreamDisconnected"
                    | "responseTooManyFailedAttempts" => Self::Network {
                        http_status: body
                            .get("httpStatusCode")
                            .and_then(Value::as_u64)
                            .and_then(|status| u16::try_from(status).ok()),
                    },
                    "activeTurnNotSteerable" => Self::ActiveTurnNotSteerable,
                    _ => Self::Other,
                }
            }
        }
    }
}

/// One entry of the model catalogue, for display only: it depends on the binary's version, not
/// on what an account may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub is_default: bool,
}

impl From<ModelDto> for ModelInfo {
    fn from(dto: ModelDto) -> Self {
        Self {
            id: dto.id,
            display_name: dto.display_name,
            is_default: dto.is_default,
        }
    }
}

/// Something the server said on its own initiative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    TurnCompleted {
        thread_id: String,
        turn: TurnRecord,
    },
    ThreadStatusChanged {
        thread_id: String,
        status: ThreadStatus,
    },
    /// The server asked a question (an approval, an elicitation) and is blocked until answered.
    /// Toglet never answers on the user's behalf; the caller interrupts the turn.
    ServerRequest {
        method: String,
    },
    /// A notification Toglet does not act on.
    Other {
        method: String,
    },
}

impl From<ThreadDto> for ThreadSummary {
    fn from(dto: ThreadDto) -> Self {
        Self {
            id: dto.id,
            cwd: dto.cwd,
            cli_version: dto.cli_version,
            created_at: dto.created_at,
            updated_at: dto.updated_at,
            title: dto.name,
            preview: dto.preview.and_then(|preview| preview.excerpt()),
            status: dto.status.into(),
            turns: dto.turns.into_iter().map(TurnRecord::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// A desktop session's recorded shape, with content fields replaced by placeholders.
    const RECORDED_THREAD: &str = r#"{"thread":{"id":"01a09301-0000-7000-8000-000000000001",
        "cwd":"/Users/someone/toglet-v02-test","cliVersion":"0.153.4","createdAt":1789100000,
        "updatedAt":1789100500,"ephemeral":false,"modelProvider":"openai","name":"Test session",
        "preview":"the first user message","path":"/Users/someone/.codex/sessions/x.jsonl",
        "projectId":null,"sessionId":"s","source":"vscode","status":{"type":"notLoaded"},
        "turns":[{"id":"t1","status":"completed","items":[{"type":"userMessage","id":"i1",
        "content":[{"type":"text","text":"secret prompt"}]}],"startedAt":1789100000,
        "completedAt":1789100010},{"id":"t2","status":"failed","items":[],
        "error":{"message":"You've hit your usage limit","codexErrorInfo":"usageLimitExceeded",
        "additionalDetails":null,"misalignment":null}}]}}"#;

    #[test]
    fn a_recorded_thread_maps_to_the_domain_form() {
        let envelope: ThreadEnvelope = parse(RECORDED_THREAD).expect("the recorded shape parses");

        let thread = ThreadSummary::from(envelope.thread);

        assert_eq!(thread.cli_version, "0.153.4");
        assert_eq!(thread.title.as_deref(), Some("Test session"));
        assert_eq!(thread.preview.as_deref(), Some("the first user message"));
        assert_eq!(thread.status, ThreadStatus::NotLoaded);
        assert_eq!(thread.folder_name().as_deref(), Some("toglet-v02-test"));
        assert_eq!(thread.turns.len(), 2);
        let last = thread.last_turn().expect("two turns were loaded");
        assert_eq!(last.status, TurnStatus::Failed);
        assert_eq!(last.error, Some(TurnErrorKind::UsageLimitExceeded));
        assert_eq!(thread.turns[0].error, None);
        assert_eq!(thread.turns[0].completed_at, Some(1_789_100_010));
    }

    #[test]
    fn session_content_and_paths_are_never_materialised() {
        let envelope: ThreadEnvelope = parse(RECORDED_THREAD).expect("payload parses");

        let captured = format!("{envelope:?}");
        for content in [
            "secret prompt",
            "the first user message",
            "usage limit",
            "sessions/x.jsonl",
        ] {
            assert!(
                !captured.contains(content),
                "`{content}` must not be captured by a thread read"
            );
        }
    }

    #[test]
    fn the_debug_form_shows_the_folder_name_and_not_the_path_or_the_preview() {
        let envelope: ThreadEnvelope = parse(RECORDED_THREAD).expect("payload parses");

        let captured = format!("{:?}", ThreadSummary::from(envelope.thread));

        assert!(captured.contains("toglet-v02-test"));
        assert!(
            !captured.contains("/Users/someone"),
            "an absolute path must not appear in the debug form: {captured}"
        );
        assert!(
            !captured.contains("first user message"),
            "the preview must not appear in the debug form: {captured}"
        );
    }

    #[test]
    fn a_preview_becomes_a_one_line_excerpt_with_a_bounded_length() {
        let one_line = Preview("  fix the\n\n  parser   please ".to_owned());
        assert_eq!(one_line.excerpt().as_deref(), Some("fix the parser please"));

        let blank = Preview(" \n\t".to_owned());
        assert_eq!(blank.excerpt(), None);

        let long = Preview("字".repeat(200));
        let excerpt = long.excerpt().expect("not blank");
        assert_eq!(excerpt.chars().count(), PREVIEW_EXCERPT_CHARS + 1);
        assert!(excerpt.ends_with('…'));

        let exact = Preview("a".repeat(PREVIEW_EXCERPT_CHARS));
        assert_eq!(
            exact.excerpt().as_deref(),
            Some("a".repeat(PREVIEW_EXCERPT_CHARS).as_str())
        );
    }

    #[test]
    fn a_thread_without_a_cli_version_is_refused_rather_than_defaulted() {
        let parsed: Result<ThreadEnvelope, _> = parse(
            r#"{"thread":{"id":"x","cwd":"/p","createdAt":1,"updatedAt":2,"status":{"type":"idle"}}}"#,
        );

        assert!(
            parsed.is_err(),
            "the version decides whether the session can be resumed; it cannot be guessed"
        );
    }

    #[test]
    fn a_thread_with_no_turns_field_has_no_turns_loaded() {
        let envelope: ThreadEnvelope = parse(
            r#"{"thread":{"id":"x","cwd":"/p","cliVersion":"0.153.4","createdAt":1,"updatedAt":2,
                "status":{"type":"idle"}}}"#,
        )
        .expect("payload parses");

        let thread = ThreadSummary::from(envelope.thread);
        assert_eq!(thread.last_turn(), None);
        assert_eq!(thread.title, None);
        assert_eq!(thread.preview, None);
    }

    #[test]
    fn an_active_thread_reports_why_it_is_waiting() {
        let status: ThreadStatusDto =
            parse(r#"{"type":"active","activeFlags":["waitingOnApproval","somethingNew"]}"#)
                .expect("payload parses");

        let status = ThreadStatus::from(status);

        assert_eq!(
            status,
            ThreadStatus::Active(vec![ActiveFlag::WaitingOnApproval, ActiveFlag::Unknown])
        );
        assert!(status.is_waiting_on_human());
        assert!(!ThreadStatus::Active(vec![]).is_waiting_on_human());
        assert!(!ThreadStatus::Idle.is_waiting_on_human());
    }

    #[test]
    fn a_status_type_this_build_has_never_seen_is_unknown_not_an_error() {
        let status: ThreadStatusDto =
            parse(r#"{"type":"hibernating"}"#).expect("a new status type still parses");

        assert_eq!(ThreadStatus::from(status), ThreadStatus::Unknown);
    }

    #[test]
    fn every_named_error_lands_on_one_variant() {
        let cases = [
            ("usageLimitExceeded", TurnErrorKind::UsageLimitExceeded),
            ("rateLimitExceeded", TurnErrorKind::RateLimitExceeded),
            (
                "contextWindowExceeded",
                TurnErrorKind::ContextWindowExceeded,
            ),
            (
                "sessionBudgetExceeded",
                TurnErrorKind::SessionBudgetExceeded,
            ),
            ("serverOverloaded", TurnErrorKind::ServerOverloaded),
            ("internalServerError", TurnErrorKind::InternalServerError),
            ("unauthorized", TurnErrorKind::Unauthorized),
            ("badRequest", TurnErrorKind::BadRequest),
            ("cyberPolicy", TurnErrorKind::Other),
            ("sandboxError", TurnErrorKind::Other),
            ("addedNextYear", TurnErrorKind::Other),
        ];
        for (name, expected) in cases {
            let dto: CodexErrorInfoDto =
                parse(&format!("\"{name}\"")).expect("a bare string parses");
            assert_eq!(TurnErrorKind::from(dto), expected, "{name}");
        }
    }

    #[test]
    fn the_object_form_of_a_network_error_keeps_its_http_status() {
        let dto: CodexErrorInfoDto =
            parse(r#"{"responseStreamDisconnected":{"httpStatusCode":502}}"#)
                .expect("the object form parses");

        assert_eq!(
            TurnErrorKind::from(dto),
            TurnErrorKind::Network {
                http_status: Some(502)
            }
        );

        let dto: CodexErrorInfoDto = parse(r#"{"httpConnectionFailed":{"httpStatusCode":null}}"#)
            .expect("a null status parses");
        assert_eq!(
            TurnErrorKind::from(dto),
            TurnErrorKind::Network { http_status: None }
        );
    }

    #[test]
    fn an_object_form_this_build_has_never_seen_is_other() {
        let dto: CodexErrorInfoDto =
            parse(r#"{"somethingStructured":{"detail":1}}"#).expect("payload parses");
        assert_eq!(TurnErrorKind::from(dto), TurnErrorKind::Other);

        let dto: CodexErrorInfoDto =
            parse(r#"{"activeTurnNotSteerable":{"turnKind":"review"}}"#).expect("payload parses");
        assert_eq!(
            TurnErrorKind::from(dto),
            TurnErrorKind::ActiveTurnNotSteerable
        );
    }

    #[test]
    fn a_failed_turn_without_a_reason_is_failed_with_an_unknown_reason() {
        let turn: TurnDto =
            parse(r#"{"id":"t","status":"failed","items":[],"error":{"message":"x"}}"#)
                .expect("payload parses");

        let record = TurnRecord::from(turn);

        assert_eq!(record.status, TurnStatus::Failed);
        assert_eq!(
            record.error, None,
            "no reason was given; none may be invented"
        );
    }

    #[test]
    fn a_turn_status_this_build_has_never_seen_is_unknown() {
        let turn: TurnDto =
            parse(r#"{"id":"t","status":"queued","items":[]}"#).expect("payload parses");
        assert_eq!(turn.status, TurnStatus::Unknown);
    }

    #[test]
    fn a_turn_without_a_status_is_refused() {
        assert!(parse::<TurnDto>(r#"{"id":"t","items":[]}"#).is_err());
    }

    #[test]
    fn the_resume_and_start_parameters_carry_no_override_fields() {
        let resume =
            serde_json::to_value(ThreadResumeParams { thread_id: "t" }).expect("serialises");
        let start = serde_json::to_value(TurnStartParams {
            thread_id: "t",
            input: [TextInput::new("continue")],
        })
        .expect("serialises");

        let keys = |value: &Value| -> Vec<String> {
            value
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect()
        };
        assert_eq!(keys(&resume), ["threadId"]);
        assert_eq!(keys(&start), ["input", "threadId"]);
        assert_eq!(
            start["input"],
            serde_json::json!([{ "type": "text", "text": "continue" }])
        );
        for forbidden in ["approvalPolicy", "sandboxPolicy", "sandbox", "cwd", "model"] {
            assert!(resume.get(forbidden).is_none(), "{forbidden} on resume");
            assert!(start.get(forbidden).is_none(), "{forbidden} on start");
        }
    }

    #[test]
    fn the_list_parameters_omit_the_filter_when_there_is_none() {
        let without = serde_json::to_value(ThreadListParams {
            cwd: None,
            limit: 5,
            sort_key: "updated_at",
        })
        .expect("serialises");
        let with = serde_json::to_value(ThreadListParams {
            cwd: Some(Path::new("/p")),
            limit: 5,
            sort_key: "updated_at",
        })
        .expect("serialises");

        assert!(without.get("cwd").is_none());
        assert_eq!(with["cwd"], "/p");
        assert_eq!(with["sortKey"], "updated_at");
    }

    #[test]
    fn a_turn_completed_notification_materialises_the_excerpt_and_nothing_else() {
        // BATCH-07 replaced `a_turn_completed_notification_is_read_without_its_items`. That test
        // guarded "no session content is materialised at all". One bounded excerpt is now
        // allowed (FR-REMOTE-016), so the guard is narrowed rather than dropped: everything
        // except the agent's own words must still be thrown away.
        let params: TurnCompletedParams = parse(
            r#"{"threadId":"th","turn":{"id":"t2","status":"completed","items":[
                {"type":"commandExecution","id":"c","command":"rm -rf /tmp/secret",
                 "cwd":"/Users/someone/private"},
                {"type":"fileChange","id":"f","diff":"--- a/secret.rs"},
                {"type":"reasoning","id":"r","text":"the private chain of thought"},
                {"type":"agentMessage","id":"m","text":"OK"}]}}"#,
        )
        .expect("payload parses");

        assert_eq!(params.thread_id, "th");
        let record = TurnRecord::from(params.turn);
        assert_eq!(record.status, TurnStatus::Completed);
        assert_eq!(record.agent_excerpt.as_deref(), Some("OK"));

        // Nothing else survived the parse, in the record or in a captured debug form.
        let captured = format!("{record:?}");
        for content in [
            "rm -rf",
            "/Users/someone",
            "secret.rs",
            "chain of thought",
            // Not even the excerpt itself: the record's debug form redacts it.
            "OK",
        ] {
            assert!(
                !captured.contains(content),
                "`{content}` must not survive into a turn record: {captured}"
            );
        }
    }

    #[test]
    fn the_last_thing_the_agent_said_is_the_one_kept() {
        let record = turn_with(
            r#"{"type":"agentMessage","id":"a","text":"first"},
               {"type":"commandExecution","id":"c","command":"ls"},
               {"type":"agentMessage","id":"b","text":"second"}"#,
        );
        assert_eq!(record.agent_excerpt.as_deref(), Some("second"));
    }

    #[test]
    fn an_agent_message_is_folded_to_one_line_and_cut_by_characters() {
        let record =
            turn_with(r#"{"type":"agentMessage","id":"a","text":"  I will\n\n  refactor   it "}"#);
        assert_eq!(record.agent_excerpt.as_deref(), Some("I will refactor it"));

        // Wide characters cost one each, so a Chinese answer is not cut three times too early.
        let wide = "字".repeat(MESSAGE_EXCERPT_CHARS + 50);
        let record = turn_with(&format!(
            r#"{{"type":"agentMessage","id":"a","text":"{wide}"}}"#
        ));
        let excerpt = record.agent_excerpt.expect("not blank");
        assert_eq!(excerpt.chars().count(), MESSAGE_EXCERPT_CHARS + 1);
        assert!(excerpt.ends_with('…'));

        // Exactly at the cap is returned whole, with no ellipsis.
        let exact = "a".repeat(MESSAGE_EXCERPT_CHARS);
        let record = turn_with(&format!(
            r#"{{"type":"agentMessage","id":"a","text":"{exact}"}}"#
        ));
        assert_eq!(record.agent_excerpt.as_deref(), Some(exact.as_str()));
    }

    /// `None`, never `Some("")`: an empty string would read as "the agent said nothing".
    #[test]
    fn nothing_to_quote_is_absent_rather_than_empty() {
        assert_eq!(
            turn_with(r#"{"type":"agentMessage","id":"a","text":" \n\t"}"#).agent_excerpt,
            None
        );
        assert_eq!(
            turn_with(r#"{"type":"commandExecution","id":"c","command":"ls"}"#).agent_excerpt,
            None
        );
        assert_eq!(turn_with("").agent_excerpt, None);

        // Items absent entirely - the shape `thread/list` returns - is also `None`.
        let dto: TurnDto = parse(r#"{"id":"t","status":"completed"}"#).expect("parses");
        assert_eq!(TurnRecord::from(dto).agent_excerpt, None);
    }

    /// The carrier type keeps the same discipline as `Preview`: it cannot quote itself.
    #[test]
    fn a_message_never_quotes_itself_in_a_debug_form() {
        let dto: TurnDto = parse(
            r#"{"id":"t","status":"completed","items":[{"type":"agentMessage","id":"m",
               "text":"do not print me"}]}"#,
        )
        .expect("parses");
        let captured = format!("{dto:?}");

        assert!(captured.contains("Message(..)"));
        assert!(!captured.contains("do not print me"), "{captured}");
    }

    /// Builds a completed turn from a list of item objects, so each test spoils one thing.
    fn turn_with(items: &str) -> TurnRecord {
        let dto: TurnDto = parse(&format!(
            r#"{{"id":"t","status":"completed","items":[{items}]}}"#
        ))
        .expect("payload parses");
        TurnRecord::from(dto)
    }

    #[test]
    fn a_model_entry_keeps_only_display_fields() {
        let result: ModelListResult = parse(
            r#"{"data":[{"id":"gpt-x","model":"gpt-x","displayName":"GPT X","isDefault":true,
                "hidden":false,"description":"d","defaultReasoningEffort":"medium",
                "supportedReasoningEfforts":[]}],"nextCursor":null}"#,
        )
        .expect("payload parses");

        let models: Vec<ModelInfo> = result.data.into_iter().map(ModelInfo::from).collect();
        assert_eq!(
            models,
            [ModelInfo {
                id: "gpt-x".to_owned(),
                display_name: "GPT X".to_owned(),
                is_default: true,
            }]
        );
    }
}
