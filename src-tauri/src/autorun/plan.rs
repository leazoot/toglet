//! The automatic-continuation plan on disk: `autorun-plan.json`.
//!
//! Kept apart from `metadata.json` because it holds `projectPath` (the only persisted absolute
//! path) and `resumeInstruction`; neither may reach logs, errors, IPC replies or metadata.
//! Writes are atomic; an unreadable file is replaced by a disabled plan without touching
//! credentials.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::machine::{Dedup, LastResult, Machine, Restored, ResultKind, State, WaitReason};
use crate::codex_home::atomic_write;
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::storage::{LoadOutcome, LoadProblem};

pub const PLAN_FILE: &str = "autorun-plan.json";

/// The version this build writes. Bump it together with a migration step.
pub const PLAN_SCHEMA_VERSION: u32 = 1;

/// Only the desktop app's own session is continued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    Desktop,
}

/// What the plan is bound to. Complete or absent, never partial.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub execution_environment: ExecutionEnvironment,
    /// The session's working directory, as `thread/list` reported it: the only absolute path
    /// this application persists.
    pub project_path: PathBuf,
    /// The folder name, for display.
    pub project_label: String,
    pub thread_id: String,
    pub thread_title: Option<String>,
    /// 1 to 2000 characters, checked at the command boundary. Never a command-line argument or
    /// an environment variable.
    pub resume_instruction: String,
    pub bound_at: String,
}

/// Hand-written so that `{:?}` shows the folder name and the length of the instruction, never
/// the path or the text.
impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Binding")
            .field("execution_environment", &self.execution_environment)
            .field("project_label", &self.project_label)
            .field("thread_id", &self.thread_id)
            .field("thread_title", &self.thread_title)
            .field(
                "resume_instruction_chars",
                &self.resume_instruction.chars().count(),
            )
            .field("bound_at", &self.bound_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub account_id: String,
    pub order: u32,
}

/// The plan file's `lastResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastResultRecord {
    pub kind: ResultKind,
    pub account_id: Option<String>,
    pub turn_id: Option<String>,
    pub code: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoRunPlan {
    pub schema_version: u32,
    pub enabled: bool,
    pub binding: Option<Binding>,
    pub participants: Vec<Participant>,
    pub state: State,
    pub generation: u64,
    pub executing_account_id: Option<String>,
    /// A stable code (`WaitReason::as_str`), never prose.
    pub wait_reason: Option<String>,
    /// Unix seconds. Unknown is `None`, never 0.
    pub expected_available_at: Option<i64>,
    pub next_check_at: Option<i64>,
    pub last_result: Option<LastResultRecord>,
    pub resume_count: u32,
    pub max_resumes: Option<u32>,
    pub deadline: Option<i64>,
    pub dedup: Dedup,
    pub updated_at: String,
}

impl AutoRunPlan {
    /// The plan nothing has been done to: disabled, unbound, nobody taking part.
    pub fn disabled(updated_at: &str) -> Self {
        Self {
            schema_version: PLAN_SCHEMA_VERSION,
            enabled: false,
            binding: None,
            participants: Vec::new(),
            state: State::Disabled,
            generation: 0,
            executing_account_id: None,
            wait_reason: None,
            expected_available_at: None,
            next_check_at: None,
            last_result: None,
            resume_count: 0,
            max_resumes: Some(super::machine::DEFAULT_MAX_RESUMES),
            deadline: None,
            dedup: Dedup::default(),
            updated_at: updated_at.to_owned(),
        }
    }

    /// Copies what the machine knows into the plan; called after every accepted event, before
    /// saving. `last_result` keeps its timestamp unless the result itself changed.
    pub fn record(&mut self, machine: &Machine, now: &str) {
        self.enabled = machine.state() != State::Disabled;
        self.state = machine.state();
        self.generation = machine.generation();
        self.executing_account_id = machine.executing_account_id().map(str::to_owned);
        self.wait_reason = machine
            .wait_reason()
            .map(|reason| reason.as_str().to_owned());
        self.expected_available_at = machine.expected_available_at();
        self.next_check_at = machine.next_check_at();
        self.resume_count = machine.resume_count();
        self.max_resumes = machine.max_resumes();
        self.dedup = machine.dedup().clone();
        self.last_result = match machine.last_result() {
            None => None,
            Some(result) => {
                let unchanged = self
                    .last_result
                    .as_ref()
                    .is_some_and(|recorded| recorded.matches(result));
                match self.last_result.take() {
                    Some(recorded) if unchanged => Some(recorded),
                    _ => Some(LastResultRecord {
                        kind: result.kind,
                        account_id: result.account_id.clone(),
                        turn_id: result.turn_id.clone(),
                        code: result.code.clone(),
                        at: now.to_owned(),
                    }),
                }
            }
        };
        self.updated_at = now.to_owned();
    }

    /// The machine this plan describes, as [`Machine::restore`] brings it back. A wait reason
    /// this build cannot read is dropped rather than guessed.
    pub fn restore_machine(&self) -> Machine {
        Machine::restore(Restored {
            state: self.state,
            generation: self.generation,
            resume_count: self.resume_count,
            max_resumes: self.max_resumes,
            executing_account_id: self.executing_account_id.clone(),
            wait_reason: self.wait_reason.as_deref().and_then(WaitReason::parse),
            dedup: self.dedup.clone(),
            last_result: self.last_result.as_ref().map(|record| LastResult {
                kind: record.kind,
                account_id: record.account_id.clone(),
                turn_id: record.turn_id.clone(),
                code: record.code.clone(),
            }),
        })
    }
}

impl LastResultRecord {
    fn matches(&self, result: &LastResult) -> bool {
        self.kind == result.kind
            && self.account_id == result.account_id
            && self.turn_id == result.turn_id
            && self.code == result.code
    }
}

pub struct PlanStore {
    path: PathBuf,
}

impl PlanStore {
    /// `directory` is the application data directory, which already exists and is private.
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(PLAN_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the plan, replacing it with a disabled one if it cannot be used. A damaged plan never
    /// blocks start-up or touches accounts and credentials; the rebuild is logged and reported.
    pub fn load(&self, now: &str) -> (AutoRunPlan, LoadOutcome) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return (AutoRunPlan::disabled(now), LoadOutcome::Created);
        };

        let parsed = read_schema_version(&text)
            .ok_or(LoadProblem::Unreadable)
            .and_then(|version| {
                if version > PLAN_SCHEMA_VERSION {
                    Err(LoadProblem::FromTheFuture { found: version })
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                serde_json::from_str::<AutoRunPlan>(&text).map_err(|_| LoadProblem::Unreadable)
            });

        match parsed {
            Ok(plan) => (plan, LoadOutcome::Loaded),
            Err(problem) => {
                log(&LogRecord::new(Level::Error, "autorun_plan_rebuilt")
                    .with_phase(Phase::Storage)
                    .with_code(ErrorCode::Internal)
                    .with_detail(match problem {
                        LoadProblem::Unreadable => "the plan file could not be parsed",
                        LoadProblem::FromTheFuture { .. } => {
                            "the plan file was written by a newer version"
                        }
                    }));
                (AutoRunPlan::disabled(now), LoadOutcome::Rebuilt { problem })
            }
        }
    }

    /// Replaces the plan. Either the whole new plan lands or the old one stays.
    pub fn save(&self, plan: &AutoRunPlan) -> Result<()> {
        let json = serde_json::to_vec_pretty(plan).map_err(|error| {
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail(&error.to_string())
        })?;

        atomic_write(&self.path, &json).map_err(|error| {
            TogletError::new(
                ErrorCode::CodexHomeUnwritable,
                Phase::Storage,
                true,
                UserAction::Retry,
            )
            .with_detail(&error.to_string())
        })
    }
}

/// Reads only `schemaVersion`. A plan from a newer build usually fails a full parse - that is
/// the point of it being newer - so the version is checked first to report the real reason.
fn read_schema_version(text: &str) -> Option<u32> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct VersionOnly {
        schema_version: u32,
    }
    serde_json::from_str::<VersionOnly>(text)
        .ok()
        .map(|only| only.schema_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autorun::machine::{Action, Fact, Interruption, Outcome, UserEvent};
    use crate::codex_home::{IsolatedHome, permissions};

    const NOW: &str = "2026-09-11T10:00:00Z";
    const LATER: &str = "2026-09-11T11:00:00Z";
    const PROJECT_PATH: &str = "/Users/someone/Projects/toglet-demo";
    const INSTRUCTION: &str = "Continue the unfinished work; check the files first.";

    fn scratch() -> IsolatedHome {
        IsolatedHome::create(Phase::Storage).expect("scratch directory")
    }

    fn binding() -> Binding {
        Binding {
            execution_environment: ExecutionEnvironment::Desktop,
            project_path: PathBuf::from(PROJECT_PATH),
            project_label: "toglet-demo".to_owned(),
            thread_id: "thread-1".to_owned(),
            thread_title: Some("Fix the parser".to_owned()),
            resume_instruction: INSTRUCTION.to_owned(),
            bound_at: NOW.to_owned(),
        }
    }

    fn bound_plan() -> AutoRunPlan {
        let mut plan = AutoRunPlan::disabled(NOW);
        plan.binding = Some(binding());
        plan.participants = vec![
            Participant {
                account_id: "a".to_owned(),
                order: 0,
            },
            Participant {
                account_id: "b".to_owned(),
                order: 1,
            },
        ];
        plan
    }

    /// A machine that has continued once and is running.
    fn running_machine() -> Machine {
        let mut machine = Machine::new(Some(8));
        machine.apply_user(UserEvent::Enable);
        let mut feed = |fact: Fact| {
            let generation = machine.generation();
            assert!(matches!(
                machine.apply_fact(generation, fact),
                Outcome::Applied(_)
            ));
        };
        feed(Fact::TurnEnded {
            turn_id: "t1".to_owned(),
            interruption: Interruption::Exhausted,
        });
        feed(Fact::Chosen(crate::autorun::Selection::Now {
            account_id: "a".to_owned(),
            reason: crate::autorun::Reason::ListPosition,
        }));
        feed(Fact::Verified {
            verdict: crate::autorun::Verdict::Available,
            active_account_id: Some("a".to_owned()),
        });
        feed(Fact::Resumed {
            turn_id: "t2".to_owned(),
        });
        assert_eq!(machine.state(), State::Running);
        machine
    }

    #[test]
    fn a_missing_file_is_a_disabled_plan() {
        let home = scratch();
        let (plan, outcome) = PlanStore::new(home.path()).load(NOW);
        assert_eq!(outcome, LoadOutcome::Created);
        assert_eq!(plan, AutoRunPlan::disabled(NOW));
        assert!(!plan.enabled);
        assert_eq!(plan.state, State::Disabled);
        assert_eq!(plan.max_resumes, Some(8));
    }

    #[test]
    fn a_saved_plan_is_read_back_whole() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        let mut plan = bound_plan();
        plan.record(&running_machine(), NOW);
        store.save(&plan).expect("saved");

        let (loaded, outcome) = store.load(LATER);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(loaded, plan);
    }

    // Permissions before content: the file is private from the moment it exists.
    #[test]
    fn the_file_is_private() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        store.save(&bound_plan()).expect("saved");
        permissions::assert_private(store.path());
    }

    #[test]
    fn saving_again_replaces_the_content_and_leaves_no_temporary_file() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        store.save(&bound_plan()).expect("first");
        let mut second = bound_plan();
        second.record(&running_machine(), LATER);
        store.save(&second).expect("second");

        assert_eq!(store.load(NOW).0, second);
        let leftovers: Vec<_> = std::fs::read_dir(home.path())
            .expect("readable")
            .map(|entry| entry.expect("entry").file_name())
            .filter(|name| name.to_string_lossy().contains(".toglet-tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    // Nothing but the plan's own vocabulary is in the file. The two sensitive fields are there
    // by design; nothing that identifies an account is.
    #[test]
    fn the_file_holds_the_binding_and_nothing_secret() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        let mut plan = bound_plan();
        plan.record(&running_machine(), NOW);
        store.save(&plan).expect("saved");

        let text = std::fs::read_to_string(store.path()).expect("readable");
        assert!(text.contains("\"projectPath\""));
        assert!(text.contains(PROJECT_PATH));
        assert!(text.contains(INSTRUCTION));
        assert!(text.contains("\"state\": \"running\""));
        assert!(text.contains("\"executionEnvironment\": \"desktop\""));
        assert!(text.contains("\"kind\": \"resumed\""));
        for forbidden in ["token", "Token", "@", "auth.json", "eyJ", "sk-"] {
            assert!(!text.contains(forbidden), "{forbidden} in the plan file");
        }
    }

    #[test]
    fn debug_output_of_a_binding_shows_the_folder_not_the_path_or_the_text() {
        let text = format!("{:?}", binding());
        assert!(text.contains("toglet-demo"));
        assert!(text.contains("thread-1"));
        assert!(!text.contains(PROJECT_PATH));
        assert!(!text.contains("someone"));
        assert!(!text.contains(INSTRUCTION));
        assert!(!text.contains("Continue"));
        assert!(text.contains("resume_instruction_chars"));

        let plan_text = format!("{:?}", bound_plan());
        assert!(!plan_text.contains(PROJECT_PATH));
        assert!(!plan_text.contains(INSTRUCTION));
    }

    #[test]
    fn an_unreadable_file_is_rebuilt_as_disabled_and_reported() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        std::fs::write(store.path(), b"{ this is not json").expect("written");

        let (plan, outcome) = store.load(NOW);
        assert_eq!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::Unreadable
            }
        );
        assert_eq!(plan, AutoRunPlan::disabled(NOW));
    }

    #[test]
    fn a_file_from_a_newer_build_is_refused_not_guessed_at() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        std::fs::write(
            store.path(),
            br#"{"schemaVersion": 99, "enabled": true, "somethingNew": {}}"#,
        )
        .expect("written");

        let (plan, outcome) = store.load(NOW);
        assert_eq!(
            outcome,
            LoadOutcome::Rebuilt {
                problem: LoadProblem::FromTheFuture { found: 99 }
            }
        );
        assert!(!plan.enabled);
    }

    #[test]
    fn a_rebuilt_plan_does_not_touch_anything_else_in_the_directory() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        let neighbour = home.path().join("metadata.json");
        std::fs::write(&neighbour, b"{\"schemaVersion\":3}").expect("written");
        std::fs::write(store.path(), b"garbage").expect("written");

        let (_, outcome) = store.load(NOW);
        assert!(matches!(outcome, LoadOutcome::Rebuilt { .. }));
        assert_eq!(
            std::fs::read(&neighbour).expect("still there"),
            b"{\"schemaVersion\":3}"
        );
    }

    #[test]
    fn recording_copies_the_machine_and_stamps_a_new_result_once() {
        let mut plan = bound_plan();
        let machine = running_machine();
        plan.record(&machine, NOW);
        assert!(plan.enabled);
        assert_eq!(plan.state, State::Running);
        assert_eq!(plan.generation, machine.generation());
        assert_eq!(plan.executing_account_id.as_deref(), Some("a"));
        assert_eq!(plan.resume_count, 1);
        assert_eq!(plan.dedup.last_resumed_turn_id.as_deref(), Some("t1"));
        assert_eq!(plan.wait_reason, None);
        assert_eq!(plan.expected_available_at, None);
        assert_eq!(plan.next_check_at, None);
        let result = plan.last_result.clone().expect("a result");
        assert_eq!(result.kind, ResultKind::Resumed);
        assert_eq!(result.turn_id.as_deref(), Some("t2"));
        assert_eq!(result.at, NOW);

        // Recording again later keeps the time the result happened.
        plan.record(&machine, LATER);
        assert_eq!(plan.updated_at, LATER);
        assert_eq!(plan.last_result.as_ref().map(|r| r.at.as_str()), Some(NOW));
    }

    #[test]
    fn recording_a_wait_writes_the_code_and_the_times() {
        let mut machine = Machine::new(Some(8));
        machine.apply_user(UserEvent::Enable);
        let generation = machine.generation();
        machine.apply_fact(
            generation,
            Fact::TurnEnded {
                turn_id: "t1".to_owned(),
                interruption: Interruption::Exhausted,
            },
        );
        let generation = machine.generation();
        machine.apply_fact(
            generation,
            Fact::Chosen(crate::autorun::Selection::Later {
                account_id: "b".to_owned(),
                available_at: 1_800_000_000,
                blockers: vec![crate::autorun::Blocker::WeeklyExhausted {
                    resets_at: Some(1_800_000_000),
                }],
                reason: crate::autorun::Reason::EarliestAvailable,
            }),
        );

        let mut plan = bound_plan();
        plan.record(&machine, NOW);
        assert_eq!(plan.state, State::WaitingQuota);
        assert_eq!(plan.wait_reason.as_deref(), Some("weekly_exhausted"));
        assert_eq!(plan.expected_available_at, Some(1_800_000_000));
        assert_eq!(plan.next_check_at, Some(1_800_000_090));
        assert_eq!(
            plan.last_result.as_ref().map(|r| r.kind),
            Some(ResultKind::Waited)
        );
    }

    // A restart comes back armed, re-reading everything before it acts, and the continuation
    // already made is still remembered.
    #[test]
    fn a_plan_saved_mid_cycle_restores_an_armed_machine() {
        let home = scratch();
        let store = PlanStore::new(home.path());
        let mut plan = bound_plan();
        plan.record(&running_machine(), NOW);
        store.save(&plan).expect("saved");

        let (loaded, _) = store.load(LATER);
        let machine = loaded.restore_machine();
        assert_eq!(machine.state(), State::Armed);
        assert_eq!(machine.wait_reason(), None);
        assert_eq!(machine.generation(), plan.generation + 1);
        assert_eq!(machine.resume_count(), 1);
        assert_eq!(machine.executing_account_id(), Some("a"));
        assert_eq!(machine.dedup().last_resumed_turn_id.as_deref(), Some("t1"));
        assert_eq!(
            machine.last_result().map(|r| r.kind),
            Some(ResultKind::Resumed)
        );

        // Recording the restored machine puts the re-arm into the file.
        let mut restarted = loaded;
        restarted.record(&machine, LATER);
        assert_eq!(restarted.state, State::Armed);
        assert_eq!(restarted.wait_reason, None);
        assert!(restarted.enabled);
    }

    #[test]
    fn a_plan_waiting_for_the_user_restores_as_it_was_with_its_reason() {
        let mut machine = running_machine();
        let generation = machine.generation();
        machine.apply_fact(generation, Fact::WaitingOnHuman);
        assert_eq!(machine.state(), State::NeedsHuman);
        let mut plan = bound_plan();
        plan.record(&machine, NOW);

        let restored = plan.restore_machine();
        assert_eq!(restored.state(), State::NeedsHuman);
        assert_eq!(restored.wait_reason(), Some(WaitReason::WaitingOnHuman));
        assert_eq!(restored.generation(), machine.generation());
    }

    #[test]
    fn an_unknown_wait_reason_is_dropped_not_guessed() {
        let mut plan = bound_plan();
        plan.state = State::NeedsHuman;
        plan.wait_reason = Some("something_from_a_newer_build".to_owned());
        let restored = plan.restore_machine();
        assert_eq!(restored.state(), State::NeedsHuman);
        assert_eq!(restored.wait_reason(), None);
    }

    #[test]
    fn a_disabled_plan_restores_a_machine_that_can_be_enabled() {
        let plan = AutoRunPlan::disabled(NOW);
        let mut machine = plan.restore_machine();
        assert_eq!(machine.state(), State::Disabled);
        assert_eq!(
            machine.apply_user(UserEvent::Enable),
            Outcome::Applied(Action::WatchThread)
        );
    }

    // The field names that carry the two sensitive values must not be spoken of by anything
    // that reaches the outside: not the logger, not the view layer the frontend mirrors.
    #[test]
    fn project_path_and_resume_instruction_are_named_nowhere_that_reaches_the_outside() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut scan = |path: PathBuf| {
            if path.extension().is_none_or(|ext| ext != "rs") {
                return;
            }
            let text = std::fs::read_to_string(&path).expect("source is readable");
            let production = text.split("#[cfg(test)]").next().unwrap_or_default();
            for forbidden in [
                "project_path",
                "projectPath",
                "resume_instruction",
                "resumeInstruction",
            ] {
                if production.contains(forbidden) {
                    offenders.push(format!("{} names {forbidden}", path.display()));
                }
            }
        };
        for entry in std::fs::read_dir(root.join("diagnostics")).expect("readable") {
            scan(entry.expect("entry").path());
        }
        scan(root.join("commands").join("views.rs"));
        assert!(offenders.is_empty(), "{offenders:?}");
    }
}
