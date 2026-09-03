//! A scriptable stand-in for `codex app-server`, used by the integration tests.
//!
//! It exists because the real server cannot be asked to produce the failures Toglet has to
//! survive: a `401`, a truncated payload, a crash mid-request, a slow reply. Those paths would
//! otherwise be untested, or tested against a real account, which is not allowed.
//!
//! The scenario is chosen by a `scenario` file inside `CODEX_HOME`. That is deliberate: the
//! command line stays a compile-time constant on the production side, so a test hook must not
//! be smuggled in as an argument.
//!
//! This is an example target. `cargo test` builds it; `tauri build` does not, so it cannot
//! reach a release artifact.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

/// How long the `slow` scenario waits. Longer than a fast reply, far below the client's
/// network deadline, so a slow server must still be reported as a success.
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
            // Accept the request and never answer, staying alive. This is the exact behaviour
            // observed for an illegal frame.
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

        // The login notification arrives after the response to `login/start`, exactly as the
        // real server sends it.
        if let Some(notification) = login_notification(&scenario, &method) {
            if writeln!(stdout, "{notification}")
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return;
            }
        }
    }
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
/// `login_success` and `login_failure` answer immediately; `login_pending` never answers, which
/// is what a user who walked away looks like.
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

/// The credential-store setting as this fake server currently holds it.
///
/// Kept in memory rather than in a file: what a real write does to `config.toml` was measured
/// against the real server, so what the tests need here is the *protocol* behaviour - version
/// tokens, conflicts, overrides - not a second TOML editor.
struct ConfigState {
    value: Option<String>,
    /// Bumped on every accepted write, so `expectedVersion` can go stale exactly as it does
    /// against the real server.
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
    // The real server rejects `params: null` on the config methods, and a fake that accepted it
    // let a wrong call through to a real machine before anyone noticed. Being strict here is
    // what makes the test able to catch it.
    // A configuration Codex cannot parse. Measured against the real server: every config
    // method answers `-32603` with no `data`, and the message carries the file's absolute
    // path. The message is reproduced in shape but not in content.
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

    let result = match (method, scenario) {
        ("initialize", _) => r#"{"userAgent":"fake-codex/9.9.9 (test harness)"}"#.to_owned(),

        ("account/login/start", _) => {
            r#"{"loginId":"login-1","authUrl":"https://auth.example.com/oauth?code_challenge=abc&state=xyz"}"#.to_owned()
        }
        ("account/login/cancel", _) => r#"{"status":"canceled"}"#.to_owned(),

        ("account/read", "unknown_fields") => {
            r#"{"account":{"type":"chatgpt","email":"tester@example.com","planType":"plus","somethingNew":1},"requiresOpenaiAuth":true,"alsoNew":true}"#.to_owned()
        }
        // A second, different account, so a test can prove two profiles are created rather
        // than one being silently reused.
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

        // No organisation-enforced configuration, which is what a real machine returned.
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
                // `value: null` removes the key - measured against the real server, and the only
                // way the protocol can express a deletion. Echoing the request rather than
                // assuming `"file"` is what lets a restore be tested at all.
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
                // `filePath` is present because the real server sends it. Toglet does not
                // model it - an absolute path must not reach a log or an error.
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
