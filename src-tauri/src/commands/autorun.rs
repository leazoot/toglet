//! Automatic continuation commands. The plan lives on the driver's thread; every change it
//! writes is announced on `autorun://state`. The project path never crosses this boundary, and
//! only threads from the last `list_threads` can be bound.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::state::{AppState, codex_home};
use super::views::ErrorView;
use crate::accounts::repository;
use crate::app_server::{AppServerClient, AppServerSession, CodexBinary, ThreadSummary};
use crate::autorun::{ACTIVE_SLICE, Clock, IDLE_SLICE};
use crate::autorun::{
    ActiveAccountRecord, AppPorts, AutoRunPlan, Binding, DriverConfig, DriverHandle,
    ExecutionEnvironment, LastResultRecord, Observation, Participant, ParticipantRecord, PlanStore,
    Services, State as PlanState, SystemClock, UserEvent, rfc3339, spawn,
};
use crate::codex_home::ServerHome;
use crate::credentials::{CredentialLock, SecretStore};
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::process::{SystemClientProbe, SystemClientRestart, SystemPowerAssertion};
use crate::storage::SwitchVerified;
use crate::switching::{NoFaults, SwitchLock};

pub const AUTORUN_STATE_EVENT: &str = "autorun://state";
/// Emitted when the scheduler records a new active account, so the interface re-reads the list.
pub const ACCOUNTS_CHANGED_EVENT: &str = "accounts://changed";

/// Continuation instruction length bounds, after trimming.
const INSTRUCTION_MIN_CHARS: usize = 1;
const INSTRUCTION_MAX_CHARS: usize = 2000;

const PHASE: Phase = Phase::Autorun;

/// A thread as the interface sees it: no path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub thread_id: String,
    pub title: Option<String>,
    /// Excerpt of the first message, to tell sessions apart; never stored in the binding.
    pub preview: Option<String>,
    /// Folder name only; the path stays in Rust.
    pub project_label: Option<String>,
    /// Unix seconds.
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListView {
    pub threads: Vec<ThreadView>,
    /// The server had more than one page, so a missing session may still exist.
    pub truncated: bool,
    /// An older Codex does not list sessions written by a newer one; lets the interface say so.
    pub runtime_version: Option<String>,
}

/// The binding without the project path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingView {
    pub execution_environment: ExecutionEnvironment,
    pub project_label: String,
    pub thread_id: String,
    pub thread_title: Option<String>,
    pub resume_instruction: String,
    pub bound_at: String,
}

/// The plan without `projectPath`; a test asserts the path is absent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoRunView {
    pub enabled: bool,
    pub binding: Option<BindingView>,
    pub participants: Vec<Participant>,
    pub state: PlanState,
    pub generation: u64,
    pub executing_account_id: Option<String>,
    pub wait_reason: Option<String>,
    pub expected_available_at: Option<i64>,
    pub next_check_at: Option<i64>,
    pub last_result: Option<LastResultRecord>,
    pub resume_count: u32,
    pub max_resumes: Option<u32>,
    pub deadline: Option<i64>,
    pub updated_at: String,
}

impl AutoRunView {
    pub fn of(plan: &AutoRunPlan) -> Self {
        Self {
            enabled: plan.enabled,
            binding: plan.binding.as_ref().map(|binding| BindingView {
                execution_environment: binding.execution_environment,
                project_label: binding.project_label.clone(),
                thread_id: binding.thread_id.clone(),
                thread_title: binding.thread_title.clone(),
                resume_instruction: binding.resume_instruction.clone(),
                bound_at: binding.bound_at.clone(),
            }),
            participants: plan.participants.clone(),
            state: plan.state,
            generation: plan.generation,
            executing_account_id: plan.executing_account_id.clone(),
            wait_reason: plan.wait_reason.clone(),
            expected_available_at: plan.expected_available_at,
            next_check_at: plan.next_check_at,
            last_result: plan.last_result.clone(),
            resume_count: plan.resume_count,
            max_resumes: plan.max_resumes,
            deadline: plan.deadline,
            updated_at: plan.updated_at.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindRequest {
    /// Must be one of the ids the last `list_threads` returned.
    pub thread_id: String,
    pub resume_instruction: String,
    /// Account ids, in priority order.
    pub participants: Vec<String>,
    pub max_resumes: Option<u32>,
    /// Unix seconds.
    pub deadline: Option<i64>,
}

/// A thread from the last listing, so the path never has to come back from the interface.
#[derive(Debug, Clone)]
struct ListedThread {
    cwd: PathBuf,
    title: Option<String>,
}

pub struct AutoRun {
    handle: Mutex<Option<DriverHandle>>,
    latest: Arc<Mutex<AutoRunPlan>>,
    listed: Mutex<HashMap<String, ListedThread>>,
}

impl AutoRun {
    /// Loads the plan and starts the driver. A driver that cannot start is logged and the
    /// autorun commands return errors; nothing else is affected.
    pub fn start(app: AppHandle, state: AppState) -> Self {
        let now = SystemClock.now();
        let store = PlanStore::new(state.data_directory());
        let (plan, outcome) = store.load(&rfc3339(now));
        if let crate::storage::LoadOutcome::Rebuilt { .. } = outcome {
            log(&LogRecord::new(Level::Warn, "autorun_plan_rebuilt_at_start").with_phase(PHASE));
        }
        let latest = Arc::new(Mutex::new(plan.clone()));

        let reported = Arc::clone(&latest);
        let announcer = app.clone();
        let observer = move |plan: &AutoRunPlan| {
            *reported
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = plan.clone();
            if announcer
                .emit(AUTORUN_STATE_EVENT, AutoRunView::of(plan))
                .is_err()
            {
                log(&LogRecord::new(Level::Warn, "autorun_state_not_delivered").with_phase(PHASE));
            }
        };
        let ports = AppPorts::new(
            Box::new(DriverServices { state, app }),
            Box::new(SystemClientProbe::new()),
            Box::new(SystemClientRestart::new()),
            Box::new(NoFaults),
            Box::new(SystemClock),
        );
        let handle = match spawn(DriverConfig {
            store,
            plan,
            ports: Box::new(ports),
            clock: Box::new(SystemClock),
            power: Box::new(SystemPowerAssertion),
            observer: Box::new(observer),
            active_slice: ACTIVE_SLICE,
            idle_slice: IDLE_SLICE,
        }) {
            Ok(handle) => Some(handle),
            Err(error) => {
                log(&LogRecord::from_error("autorun_driver_not_started", &error));
                None
            }
        };

        Self {
            handle: Mutex::new(handle),
            latest,
            listed: Mutex::new(HashMap::new()),
        }
    }

    pub fn latest(&self) -> AutoRunPlan {
        self.latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Forwards an observation; a driver that did not start is silently skipped.
    pub fn observe(&self, observation: Observation) {
        if let Some(handle) = self.handle().as_ref()
            && let Err(error) = handle.observe(observation)
        {
            log(&LogRecord::from_error(
                "autorun_observation_not_delivered",
                &error,
            ));
        }
    }

    fn handle(&self) -> std::sync::MutexGuard<'_, Option<DriverHandle>> {
        self.handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Delivers a user event. Both the panel commands and `remote_poll` use this single path.
    pub(crate) fn user(&self, event: UserEvent) -> Result<()> {
        self.handle()
            .as_ref()
            .ok_or_else(driver_unavailable)?
            .user(event)
    }
}

/// `AppState` plus the ability to tell the interface the active account changed.
struct DriverServices {
    state: AppState,
    app: AppHandle,
}

impl Services for DriverServices {
    fn secrets(&self) -> &dyn SecretStore {
        self.state.secrets()
    }

    fn switch_lock(&self) -> &SwitchLock {
        self.state.switch_lock()
    }

    fn credential_lock(&self) -> &CredentialLock {
        self.state.credential_lock()
    }

    fn journal_directory(&self) -> &Path {
        self.state.journal_directory()
    }

    fn default_home(&self) -> Result<PathBuf> {
        self.state.default_home()
    }

    fn binary(&self) -> Result<CodexBinary> {
        self.state.binary()
    }

    fn participant(&self, account_id: &str) -> Option<ParticipantRecord> {
        self.state.participant(account_id)
    }

    fn active(&self) -> Option<ActiveAccountRecord> {
        self.state.active()
    }

    fn record_active(&self, account_id: &str, verified: &SwitchVerified) -> Result<()> {
        self.state.record_active(account_id, verified)?;
        // Without this the interface keeps showing the old account as in use.
        if self.app.emit(ACCOUNTS_CHANGED_EVENT, ()).is_err() {
            log(&LogRecord::new(Level::Warn, "accounts_changed_not_delivered").with_phase(PHASE));
        }
        Ok(())
    }

    fn reopen_after_switch(&self) -> bool {
        self.state.reopen_after_switch()
    }
}

impl Services for AppState {
    fn secrets(&self) -> &dyn SecretStore {
        AppState::secrets(self)
    }

    fn switch_lock(&self) -> &SwitchLock {
        AppState::switch_lock(self)
    }

    fn credential_lock(&self) -> &CredentialLock {
        AppState::credential_lock(self)
    }

    fn journal_directory(&self) -> &Path {
        self.data_directory()
    }

    fn default_home(&self) -> Result<PathBuf> {
        codex_home()
    }

    fn binary(&self) -> Result<CodexBinary> {
        CodexBinary::resolve(PHASE)
    }

    fn participant(&self, account_id: &str) -> Option<ParticipantRecord> {
        self.read_document(|document| {
            repository::find(document, account_id).map(|profile| ParticipantRecord {
                credential_ref: profile.credential_ref.clone(),
                fingerprint: profile.account_fingerprint.clone(),
                status: profile.status,
                created_at: profile.created_at.clone(),
            })
        })
    }

    fn active(&self) -> Option<ActiveAccountRecord> {
        self.read_document(|document| {
            let id = document.settings.active_account_id()?;
            let profile = repository::find(document, id)?;
            Some(ActiveAccountRecord {
                account_id: profile.id.clone(),
                credential_ref: profile.credential_ref.clone(),
                fingerprint: profile.account_fingerprint.clone(),
            })
        })
    }

    fn record_active(&self, account_id: &str, verified: &SwitchVerified) -> Result<()> {
        self.with_document(|document| {
            document
                .settings
                .set_active_account_id(Some(account_id.to_owned()), verified);
            Ok(((), true))
        })
    }

    fn reopen_after_switch(&self) -> bool {
        self.read_document(|document| document.settings.reopen_codex_after_switch)
    }
}

/// Never blocks on the driver.
#[tauri::command]
pub fn read_autorun(autorun: State<'_, AutoRun>) -> AutoRunView {
    AutoRunView::of(&autorun.latest())
}

/// Lists sessions from an app server on the user's Codex home, keeping paths for `bind_autorun`.
// `async`: starting an app server takes seconds, and the main thread is the event loop.
#[tauri::command(async)]
pub fn list_threads(autorun: State<'_, AutoRun>) -> std::result::Result<ThreadListView, ErrorView> {
    reported("autorun_list_threads_failed", list(&autorun))
}

/// Logs the redacted failure before it reaches the interface, which only shows the code.
fn reported<T>(event: &'static str, result: Result<T>) -> std::result::Result<T, ErrorView> {
    result.map_err(|error| {
        log(&LogRecord::from_error(event, &error));
        ErrorView::from(error)
    })
}

fn list(autorun: &AutoRun) -> Result<ThreadListView> {
    let binary = CodexBinary::resolve(PHASE)?;
    let home = ServerHome::Default {
        path: codex_home()?,
        phase: PHASE,
    };
    let mut session = AppServerSession::open(AppServerClient::start(&binary, home)?)?;
    let page = session.list_threads(None);
    let runtime_version = session.runtime_version().map(str::to_owned);
    // Closed on both paths, so a failed listing still leaves no subprocess behind.
    let closed = session.close();
    let page = page?;
    closed?;

    let mut listed = autorun
        .listed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    listed.clear();
    let threads = page
        .threads
        .iter()
        .filter(|thread| resumable(thread))
        .map(|thread| {
            listed.insert(
                thread.id.clone(),
                ListedThread {
                    cwd: thread.cwd.clone(),
                    title: thread.title.clone(),
                },
            );
            ThreadView {
                thread_id: thread.id.clone(),
                title: thread.title.clone(),
                preview: thread.preview.clone(),
                project_label: thread.folder_name(),
                updated_at: thread.updated_at,
            }
        })
        .collect();
    Ok(ThreadListView {
        threads,
        truncated: page.truncated,
        runtime_version,
    })
}

/// A session is resumable only if its project folder still exists; other tools leave many
/// sessions in long-gone temporary directories.
fn resumable(thread: &ThreadSummary) -> bool {
    thread.cwd.is_dir()
}

/// Binds session, instruction, participants and limits in one step.
#[tauri::command]
pub fn bind_autorun(
    autorun: State<'_, AutoRun>,
    state: State<'_, AppState>,
    request: BindRequest,
) -> std::result::Result<(), ErrorView> {
    reported("autorun_bind_failed", bind(&autorun, &state, request))
}

fn bind(autorun: &AutoRun, state: &AppState, request: BindRequest) -> Result<()> {
    let now = SystemClock.now();
    let instruction = validate_instruction(&request.resume_instruction)?;
    let participants = validate_participants(state, &request.participants)?;
    if request.max_resumes == Some(0) {
        return Err(invalid("the resume cap must be at least one"));
    }
    if request.deadline.is_some_and(|deadline| deadline <= now) {
        return Err(invalid("the deadline has already passed"));
    }
    let listed = autorun
        .listed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&request.thread_id)
        .cloned()
        .ok_or_else(|| {
            TogletError::new(
                ErrorCode::ThreadUnavailable,
                PHASE,
                false,
                UserAction::RebindSession,
            )
            .with_detail("the thread is not in the last listing")
        })?;

    let binding = Binding {
        execution_environment: ExecutionEnvironment::Desktop,
        project_label: listed
            .cwd
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        project_path: listed.cwd,
        thread_id: request.thread_id,
        thread_title: listed.title,
        resume_instruction: instruction,
        bound_at: rfc3339(now),
    };
    let max_resumes = request.max_resumes;
    let deadline = request.deadline;
    autorun
        .handle()
        .as_ref()
        .ok_or_else(driver_unavailable)?
        .edit(Box::new(move |plan| {
            plan.binding = Some(binding);
            plan.participants = participants;
            plan.max_resumes = max_resumes;
            plan.deadline = deadline;
        }))
}

/// Turning off is a cancel; the binding is kept.
#[tauri::command]
pub fn set_autorun_enabled(
    autorun: State<'_, AutoRun>,
    enabled: bool,
) -> std::result::Result<(), ErrorView> {
    let result = if enabled {
        let latest = autorun.latest();
        if latest.binding.is_none() || latest.participants.is_empty() {
            Err(invalid("the plan is not complete"))
        } else {
            autorun.user(UserEvent::Enable)
        }
    } else {
        autorun.user(UserEvent::Cancel)
    };
    reported("autorun_set_enabled_failed", result)
}

#[tauri::command]
pub fn pause_autorun(autorun: State<'_, AutoRun>) -> std::result::Result<(), ErrorView> {
    reported("autorun_pause_failed", autorun.user(UserEvent::Pause))
}

#[tauri::command]
pub fn resume_autorun(autorun: State<'_, AutoRun>) -> std::result::Result<(), ErrorView> {
    reported("autorun_resume_failed", autorun.user(UserEvent::Resume))
}

#[tauri::command]
pub fn cancel_autorun(autorun: State<'_, AutoRun>) -> std::result::Result<(), ErrorView> {
    reported("autorun_cancel_failed", autorun.user(UserEvent::Cancel))
}

/// The text is user data and never reaches a log or an error.
fn validate_instruction(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let length = trimmed.chars().count();
    if (INSTRUCTION_MIN_CHARS..=INSTRUCTION_MAX_CHARS).contains(&length) {
        Ok(trimmed.to_owned())
    } else {
        Err(invalid(
            "the instruction must be 1 to 2000 characters after trimming",
        ))
    }
}

/// At least one, all known, none twice, in priority order.
fn validate_participants(state: &AppState, ids: &[String]) -> Result<Vec<Participant>> {
    if ids.is_empty() {
        return Err(invalid("at least one participating account is required"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut participants = Vec::with_capacity(ids.len());
    for (order, id) in ids.iter().enumerate() {
        if !seen.insert(id.as_str()) {
            return Err(invalid("an account is listed twice"));
        }
        let known = state.read_document(|document| repository::find(document, id).is_some());
        if !known {
            return Err(invalid("a participating account does not exist"));
        }
        participants.push(Participant {
            account_id: id.clone(),
            order: u32::try_from(order).unwrap_or(u32::MAX),
        });
    }
    Ok(participants)
}

fn invalid(detail: &str) -> TogletError {
    TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None).with_detail(detail)
}

fn driver_unavailable() -> TogletError {
    TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
        .with_detail("automatic continuation is not running")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT_PATH: &str = "/Users/somebody/Projects/toglet-demo";

    fn bound() -> AutoRunPlan {
        let mut plan = AutoRunPlan::disabled("2026-09-11T00:00:00Z");
        plan.binding = Some(Binding {
            execution_environment: ExecutionEnvironment::Desktop,
            project_path: PathBuf::from(PROJECT_PATH),
            project_label: "toglet-demo".to_owned(),
            thread_id: "thread-1".to_owned(),
            thread_title: Some("Fix the parser".to_owned()),
            resume_instruction: "Continue the work.".to_owned(),
            bound_at: "2026-09-11T00:00:00Z".to_owned(),
        });
        plan
    }

    #[test]
    fn the_view_carries_the_folder_name_and_never_the_path() {
        let json = serde_json::to_string(&AutoRunView::of(&bound())).expect("serialises");
        assert!(!json.contains("projectPath"), "{json}");
        assert!(!json.contains(PROJECT_PATH), "{json}");
        assert!(!json.contains("somebody"), "{json}");
        assert!(json.contains("\"projectLabel\":\"toglet-demo\""));
        assert!(json.contains("\"threadId\":\"thread-1\""));
        assert!(json.contains("\"resumeInstruction\":\"Continue the work.\""));
        assert!(json.contains("\"state\":\"disabled\""));
        assert!(json.contains("\"expectedAvailableAt\":null"));
    }

    #[test]
    fn a_session_whose_folder_is_gone_is_not_offered() {
        let scratch = crate::codex_home::IsolatedHome::create(PHASE).expect("a scratch directory");
        let thread = |cwd: PathBuf| ThreadSummary {
            id: "thread".to_owned(),
            cwd,
            cli_version: "0.153.4".to_owned(),
            created_at: 0,
            updated_at: 0,
            title: None,
            preview: None,
            status: crate::app_server::ThreadStatus::NotLoaded,
            turns: Vec::new(),
        };

        assert!(resumable(&thread(scratch.path().to_path_buf())));
        assert!(!resumable(&thread(scratch.path().join("gone"))));
    }

    #[test]
    fn the_listing_carries_the_preview_and_the_plan_does_not() {
        let view = ThreadView {
            thread_id: "thread-1".to_owned(),
            title: None,
            preview: Some("fix the parser".to_owned()),
            project_label: Some("toglet-demo".to_owned()),
            updated_at: 0,
        };
        let json = serde_json::to_string(&view).expect("serialises");
        assert!(json.contains("\"preview\":\"fix the parser\""), "{json}");

        let json = serde_json::to_string(&AutoRunView::of(&bound())).expect("serialises");
        assert!(!json.contains("preview"), "{json}");
    }

    #[test]
    fn the_instruction_is_trimmed_and_bounded() {
        assert_eq!(validate_instruction("  go on  ").expect("valid"), "go on");
        assert!(validate_instruction("   ").is_err());
        assert!(validate_instruction("").is_err());
        let longest = "x".repeat(INSTRUCTION_MAX_CHARS);
        assert!(validate_instruction(&longest).is_ok());
        let too_long = "x".repeat(INSTRUCTION_MAX_CHARS + 1);
        assert!(validate_instruction(&too_long).is_err());
        // Characters, not bytes.
        let wide = "字".repeat(INSTRUCTION_MAX_CHARS);
        assert!(validate_instruction(&wide).is_ok());
    }

    #[test]
    fn a_bind_request_refuses_unknown_fields() {
        let json =
            r#"{"threadId":"t","resumeInstruction":"go","participants":["a"],"projectPath":"/x"}"#;
        assert!(serde_json::from_str::<BindRequest>(json).is_err());
        let json = r#"{"threadId":"t","resumeInstruction":"go","participants":["a"]}"#;
        let request: BindRequest = serde_json::from_str(json).expect("parses");
        assert_eq!(request.max_resumes, None);
        assert_eq!(request.deadline, None);
    }
}
