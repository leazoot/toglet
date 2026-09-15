//! A scriptable stand-in for `codex app-server`, used by the integration tests.
//!
//! The scenario comes from a `scenario` file inside `CODEX_HOME`, so the production command line
//! stays constant. An example target: `cargo test` builds it, `tauri build` does not.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

/// How long the `slow` scenario waits; far below the client's deadline, so it must still succeed.
const SLOW_REPLY: Duration = Duration::from_millis(1200);

fn main() {
    let scenario = scenario();
    let mut config = ConfigState::new(&scenario);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { return };
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
            // Mirrors the real server: a malformed frame draws no reply and no exit.
            continue;
        };
        let method = request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(id) = request.get("id").and_then(serde_json::Value::as_u64) else {
            // A notification. Nothing to answer.
            continue;
        };

        match scenario.as_str() {
            // Accept the request and never answer while staying alive, as the real server does
            // for an illegal frame.
            "timeout" => continue,
            "crash" if method != "initialize" => std::process::exit(3),
            "slow" => std::thread::sleep(SLOW_REPLY),
            _ => {}
        }

        let params = request
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let reply = reply(&scenario, &method, id, &params, &mut config);
        if writeln!(stdout, "{reply}")
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return;
        }

        // Notifications and server requests follow the response, as with the real server.
        for frame in follow_ups(&scenario, &method) {
            if writeln!(stdout, "{frame}")
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return;
            }
        }
    }
}

/// Frames a scenario sends on its own after answering `method`, in order.
fn follow_ups(scenario: &str, method: &str) -> Vec<String> {
    let mut frames = Vec::new();
    frames.extend(login_notification(scenario, method));
    frames.extend(turn_follow_ups(scenario, method));
    frames
}

/// The one thread every thread scenario is about.
const THREAD_ID: &str = "01a09301-0000-7000-8000-000000000001";
/// The turn `turn/start` accepts.
const TURN_ID: &str = "01a09302-0000-7000-8000-000000000002";

/// The threads `thread/list` knows: `(id, cwd, name)`, across three projects so a cwd filter
/// visibly selects.
fn thread_catalogue(scenario: &str) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut threads = vec![(THREAD_ID, "/fake/project-a", "Fake session")];
    if scenario == "multi_thread" {
        threads.push((
            "01a09301-0000-7000-8000-000000000002",
            "/fake/project-b",
            "Second project",
        ));
        threads.push((
            "01a09301-0000-7000-8000-000000000003",
            "/fake/project-c",
            "Third project",
        ));
    }
    threads
}

/// A `Thread` in the shape a 0.153.4 server returns, content fields included so the client can
/// be seen to drop them.
fn thread_json(id: &str, cwd: &str, name: &str, status: &str, turns: &str) -> String {
    format!(
        r#"{{"id":"{id}","cwd":"{cwd}","cliVersion":"9.9.9","createdAt":1789100000,"updatedAt":1789100500,"ephemeral":false,"modelProvider":"openai","name":"{name}","preview":"the first user message","path":"/fake/sessions/{id}.jsonl","projectId":null,"sessionId":"s","source":"vscode","status":{status},"turns":{turns}}}"#
    )
}

fn turn_json(id: &str, status: &str, error: Option<&str>) -> String {
    let completed_at = if status == "inProgress" {
        "null"
    } else {
        "1789100010"
    };
    let error = error.unwrap_or("null");
    format!(
        r#"{{"id":"{id}","status":"{status}","items":[{{"type":"userMessage","id":"i","content":[{{"type":"text","text":"secret prompt"}}]}}],"startedAt":1789100000,"completedAt":{completed_at},"error":{error}}}"#
    )
}

/// The turns a scenario's thread already has when it is read or resumed.
fn turns_json(scenario: &str) -> String {
    let last = match scenario {
        "usage_limit_turn" => turn_json("t2", "failed", completed_turn_error(scenario)),
        "turn_in_progress" => turn_json("t2", "inProgress", None),
        _ => turn_json("t2", "completed", None),
    };
    format!("[{},{last}]", turn_json("t1", "completed", None))
}

fn thread_status_json(scenario: &str) -> &'static str {
    match scenario {
        "turn_in_progress" => r#"{"type":"active","activeFlags":[]}"#,
        _ => r#"{"type":"notLoaded"}"#,
    }
}

/// The `TurnError` a scenario's turn ends with, in the 0.153.4 schema's shapes: a bare string,
/// or a single-key object for values that carry data.
fn completed_turn_error(scenario: &str) -> Option<&'static str> {
    match scenario {
        "usage_limit_turn" => Some(
            r#"{"message":"You've hit your usage limit.","codexErrorInfo":"usageLimitExceeded","additionalDetails":null,"misalignment":null}"#,
        ),
        "unauthorized_turn" => {
            Some(r#"{"message":"Unauthorized","codexErrorInfo":"unauthorized"}"#)
        }
        "network_turn" => Some(
            r#"{"message":"connection failed","codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":502}}}"#,
        ),
        // From the schema: `turn/start` against a turn that cannot be steered.
        "turn_in_progress" => Some(
            r#"{"message":"active turn cannot be steered","codexErrorInfo":{"activeTurnNotSteerable":{"turnKind":"review"}}}"#,
        ),
        _ => None,
    }
}

/// What the server says on its own after a turn is started or interrupted.
fn turn_follow_ups(scenario: &str, method: &str) -> Vec<String> {
    let status_changed = |status: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","method":"thread/status/changed","params":{{"threadId":"{THREAD_ID}","status":{status}}}}}"#
        )
    };
    let completed = |status: &str, error: Option<&str>| {
        format!(
            r#"{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"{THREAD_ID}","turn":{}}}}}"#,
            turn_json(TURN_ID, status, error)
        )
    };

    match (method, scenario) {
        // The model asked a question. A server-initiated *request* follows: it carries an id
        // and the turn stays blocked until somebody answers it.
        ("turn/start", "waiting_on_user_input") => vec![
            status_changed(r#"{"type":"active","activeFlags":["waitingOnUserInput"]}"#),
            format!(
                r#"{{"jsonrpc":"2.0","id":900,"method":"item/tool/requestUserInput","params":{{"threadId":"{THREAD_ID}","turnId":"{TURN_ID}","questions":[{{"id":"q1","header":"Which one?"}}]}}}}"#
            ),
        ],
        ("turn/interrupt", "waiting_on_user_input") => vec![
            completed("interrupted", None),
            status_changed(r#"{"type":"idle"}"#),
        ],
        (
            "turn/start",
            "turn_completed" | "usage_limit_turn" | "unauthorized_turn" | "network_turn"
            | "turn_in_progress",
        ) => {
            let error = completed_turn_error(scenario);
            let status = if error.is_some() {
                "failed"
            } else {
                "completed"
            };
            vec![
                status_changed(r#"{"type":"active","activeFlags":[]}"#),
                completed(status, error),
                status_changed(r#"{"type":"idle"}"#),
            ]
        }
        _ => Vec::new(),
    }
}

/// The thread, turn and model methods. `None` when `method` is none of them.
fn thread_reply(
    scenario: &str,
    method: &str,
    id: u64,
    params: &serde_json::Value,
) -> Option<String> {
    // In 0.153.4 a thread the server cannot serve answers `-32600` with no `data` and a message
    // that quotes the id.
    let unavailable = |message: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32600,"message":"{message}: {THREAD_ID}"}}}}"#
        )
    };

    let result = match (method, scenario) {
        ("thread/list", _) => {
            let filter = params.get("cwd").and_then(serde_json::Value::as_str);
            let threads: Vec<String> = thread_catalogue(scenario)
                .into_iter()
                .filter(|(_, cwd, _)| filter.is_none_or(|filter| filter == *cwd))
                .map(|(id, cwd, name)| thread_json(id, cwd, name, r#"{"type":"notLoaded"}"#, "[]"))
                .collect();
            format!(r#"{{"data":[{}],"nextCursor":null}}"#, threads.join(","))
        }
        ("thread/read", "resume_rejected") => return Some(unavailable("thread not loaded")),
        ("thread/read", _) => format!(
            r#"{{"thread":{}}}"#,
            thread_json(
                THREAD_ID,
                "/fake/project-a",
                "Fake session",
                thread_status_json(scenario),
                &turns_json(scenario),
            )
        ),
        ("thread/resume", "resume_rejected") => {
            return Some(unavailable("no rollout found for thread id"));
        }
        ("thread/resume", _) => format!(
            r#"{{"thread":{},"approvalPolicy":"never","approvalsReviewer":"user","cwd":"/fake/project-a","model":"gpt-fake","modelProvider":"openai","sandbox":{{"type":"readOnly"}}}}"#,
            thread_json(
                THREAD_ID,
                "/fake/project-a",
                "Fake session",
                r#"{"type":"idle"}"#,
                &turns_json(scenario),
            )
        ),
        ("turn/start", _) => format!(r#"{{"turn":{}}}"#, turn_json(TURN_ID, "inProgress", None)),
        ("turn/interrupt", _) => "{}".to_owned(),
        ("model/list", _) => {
            r#"{"data":[{"id":"gpt-fake","model":"gpt-fake","displayName":"GPT Fake","isDefault":true,"hidden":false,"description":"d","defaultReasoningEffort":"medium","supportedReasoningEfforts":[]}],"nextCursor":null}"#.to_owned()
        }
        _ => return None,
    };
    Some(format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#
    ))
}

fn scenario() -> String {
    let Some(home) = std::env::var_os("CODEX_HOME").map(PathBuf::from) else {
        return "normal".to_owned();
    };
    std::fs::read_to_string(home.join("scenario"))
        .map(|scenario| scenario.trim().to_owned())
        .unwrap_or_else(|_| "normal".to_owned())
}

/// The `account/login/completed` notification a scenario sends, if any.
///
/// `login_pending` never answers, like a user who walked away.
fn login_notification(scenario: &str, method: &str) -> Option<String> {
    if method != "account/login/start" {
        return None;
    }
    let success = match scenario {
        "login_success" => "true",
        "login_failure" | "login_cancel" => "false",
        _ => return None,
    };
    Some(format!(
        r#"{{"jsonrpc":"2.0","method":"account/login/completed","params":{{"loginId":"login-1","success":{success}}}}}"#
    ))
}

fn is_config_method(method: &str) -> bool {
    matches!(
        method,
        "config/read" | "config/value/write" | "configRequirements/read"
    )
}

/// The credential-store setting as this fake server holds it, in memory: the tests need the
/// protocol behaviour (version tokens, conflicts, overrides), not a TOML editor.
struct ConfigState {
    value: Option<String>,
    /// Bumped on every accepted write, so `expectedVersion` can go stale.
    version: u32,
    /// `false` for the scenario where a write reports success without taking effect.
    writes_take_effect: bool,
}

impl ConfigState {
    fn new(scenario: &str) -> Self {
        Self {
            value: match scenario {
                "config_already_file" | "config_managed_layer" | "config_restore_ineffective" => {
                    Some("file".to_owned())
                }
                "config_other_value" => Some("keychain".to_owned()),
                _ => None,
            },
            version: 1,
            writes_take_effect: !matches!(
                scenario,
                "config_write_ineffective" | "config_restore_ineffective"
            ),
        }
    }

    fn version_token(&self) -> String {
        format!("sha256:fake{:04}", self.version)
    }
}

fn reply(
    scenario: &str,
    method: &str,
    id: u64,
    params: &serde_json::Value,
    config: &mut ConfigState,
) -> String {
    // A configuration Codex cannot parse: every config method answers `-32603` with no `data`,
    // and the real message carries the file's absolute path.
    if scenario == "config_broken" && is_config_method(method) {
        return format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32603,"message":"failed to read configuration layers: /fake/config.toml:3:16: unclosed table, expected `]`"}}}}"#
        );
    }

    if method.starts_with("config") && params.is_null() {
        return format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32600,"message":"Invalid request: expected params object"}}}}"#
        );
    }

    if scenario == "unauthorized" && method != "initialize" {
        return format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":401,"message":"Unauthorized"}}}}"#
        );
    }

    if let Some(reply) = thread_reply(scenario, method, id, params) {
        return reply;
    }

    let result = match (method, scenario) {
        ("initialize", _) => r#"{"userAgent":"fake-codex/9.9.9 (test harness)"}"#.to_owned(),

        ("account/login/start", _) => {
            r#"{"loginId":"login-1","authUrl":"https://auth.example.com/oauth?code_challenge=abc&state=xyz"}"#.to_owned()
        }
        ("account/login/cancel", _) => r#"{"status":"canceled"}"#.to_owned(),

        ("account/read", "unknown_fields") => {
            r#"{"account":{"type":"chatgpt","email":"tester@example.com","planType":"plus","somethingNew":1},"requiresOpenaiAuth":true,"alsoNew":true}"#.to_owned()
        }
        // A second, different account, so a test can prove two profiles are created.
        ("account/read", "second_account") => {
            r#"{"account":{"type":"chatgpt","email":"other@example.com","planType":"pro"},"requiresOpenaiAuth":true}"#.to_owned()
        }
        ("account/read", "api_key_account") => {
            r#"{"account":{"type":"apiKey"},"requiresOpenaiAuth":true}"#.to_owned()
        }
        ("account/read", "signed_out") => r#"{"account":null,"requiresOpenaiAuth":true}"#.to_owned(),
        ("account/read", _) => {
            r#"{"account":{"type":"chatgpt","email":"tester@example.com","planType":"plus"},"requiresOpenaiAuth":true}"#.to_owned()
        }

        // A window without `usedPercent`. The client must refuse it rather than substitute 0.
        ("account/rateLimits/read", "missing_field") => {
            r#"{"rateLimits":{"primary":{"windowDurationMins":300,"resetsAt":1788164992}}}"#
                .to_owned()
        }
        ("account/rateLimits/read", "unknown_fields") => {
            r#"{"rateLimits":{"primary":{"usedPercent":2,"windowDurationMins":300,"resetsAt":1788164992,"newField":"x"},"credits":{"hasCredits":false,"unlimited":false,"balance":"0"},"planType":"plus","anotherNewThing":[1,2]}}"#.to_owned()
        }
        ("account/rateLimits/read", _) => {
            r#"{"rateLimits":{"primary":{"usedPercent":2,"windowDurationMins":300,"resetsAt":1788164992},"secondary":{"usedPercent":0,"windowDurationMins":10080,"resetsAt":1788751792},"planType":"plus"}}"#.to_owned()
        }

        // No organisation-enforced configuration.
        ("configRequirements/read", "config_org_enforced") => {
            r#"{"requirements":{"allowedApprovalPolicies":["never"]}}"#.to_owned()
        }
        ("configRequirements/read", _) => r#"{"requirements":null}"#.to_owned(),

        ("config/read", _) => {
            let layer = if scenario == "config_managed_layer" {
                "legacyManagedConfigTomlFromMdm"
            } else {
                "user"
            };
            match &config.value {
                Some(value) => format!(
                    r#"{{"config":{{"cli_auth_credentials_store":"{value}","model":"gpt-5.6-sol"}},"origins":{{"cli_auth_credentials_store":{{"name":{{"type":"{layer}"}},"version":"{}"}}}}}}"#,
                    config.version_token()
                ),
                // No layer claims the key, so there is no origin entry for it either.
                None => r#"{"config":{"model":"gpt-5.6-sol"},"origins":{}}"#.to_owned(),
            }
        }

        ("config/value/write", "config_version_conflict") => {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32600,"message":"Configuration was modified since last read.","data":{{"config_write_error_code":"configVersionConflict"}}}}}}"#
            );
        }
        ("config/value/write", "config_layer_readonly") => {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32600,"message":"Layer is read only.","data":{{"config_write_error_code":"configLayerReadonly"}}}}}}"#
            );
        }
        ("config/value/write", "config_unknown_key") => {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32600,"message":"Unknown key.","data":{{"config_write_error_code":"configSchemaUnknownKey"}}}}}}"#
            );
        }
        ("config/value/write", "config_unmapped_error") => {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32600,"message":"Nope.","data":{{"config_write_error_code":"userLayerNotFound"}}}}}}"#
            );
        }
        ("config/value/write", _) => {
            if config.writes_take_effect {
                // `value: null` is the protocol's only way to delete a key. The request is echoed
                // rather than assuming `"file"` so a restore can be tested.
                config.value = params
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                config.version += 1;
            }
            let overridden = if scenario == "config_overridden" {
                r#"{"type":"legacyManagedConfigTomlFromMdm"}"#
            } else {
                "null"
            };
            format!(
                // The real server sends `filePath`; Toglet ignores it, since an absolute path must
                // not reach a log or an error.
                r#"{{"status":"ok","version":"{}","filePath":"/fake/config.toml","overriddenMetadata":{overridden}}}"#,
                config.version_token()
            )
        }

        _ => {
            return format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"Method not found"}}}}"#
            );
        }
    };

    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#)
}
