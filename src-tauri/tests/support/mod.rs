//! Shared helpers for the app server scenario tests.
// Compiled into every integration test binary, each of which uses only some helpers.
#![allow(dead_code)]

use std::path::PathBuf;

use toglet_lib::app_server::{AppServerClient, CodexBinary};
use toglet_lib::codex_home::IsolatedHome;
use toglet_lib::diagnostics::Phase;

/// Locates the `fake_app_server` example next to the test binary.
///
/// `CARGO_BIN_EXE_*` only covers `[[bin]]` targets; `target/<profile>/deps/<test>` sits one level
/// below `target/<profile>`.
pub fn fake_server_binary() -> PathBuf {
    let mut directory = std::env::current_exe().expect("the test binary has a path");
    directory.pop();
    if directory.ends_with("deps") {
        directory.pop();
    }
    let binary = directory
        .join("examples")
        .join(format!("fake_app_server{}", std::env::consts::EXE_SUFFIX));
    assert!(
        binary.is_file(),
        "the fake app server example was not built; `cargo test` builds examples"
    );
    binary
}

/// A fresh isolated home with the fake server's scenario in it, so the production command line
/// stays constant.
pub fn scenario_home(scenario: &str, phase: Phase) -> IsolatedHome {
    let home = IsolatedHome::create(phase).expect("isolated home is created");
    std::fs::write(home.path().join("scenario"), scenario).expect("scenario is written");
    home
}

/// The verified path to the fake server, ready to be started.
pub fn fake_binary(phase: Phase) -> CodexBinary {
    CodexBinary::at(fake_server_binary(), phase).expect("the fake server is a real file")
}

/// Starts the fake server in a fresh isolated home running `scenario`.
pub fn start_scenario(scenario: &str) -> AppServerClient {
    let home = scenario_home(scenario, Phase::ReadQuota);
    AppServerClient::start(&fake_binary(Phase::ReadQuota), home).expect("the fake server starts")
}
