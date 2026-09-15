// Suppress the extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), toglet_lib::StartupFailure> {
    toglet_lib::run()
}
