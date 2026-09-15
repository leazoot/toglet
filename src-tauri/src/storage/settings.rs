//! Application settings and their defaults.

use serde::{Deserialize, Serialize};

/// Refresh interval bounds.
const MIN_REFRESH_SECONDS: u32 = 30;
const MAX_REFRESH_SECONDS: u32 = 3600;

const DEFAULT_ACTIVE_REFRESH_SECONDS: u32 = 60;
const DEFAULT_INACTIVE_REFRESH_SECONDS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockEdge {
    Left,
    Right,
}

/// What the collapsed surface is: the 60 × 168 bar or the shorter 60 × 98 ring form.
///
/// `window::geometry` reads the height from this so the hover target follows the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockShape {
    Bar,
    Ring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Dark,
    Light,
}

/// Which language the interface is labelled in.
///
/// `System` means no choice has been made yet; the interface only offers English and Chinese.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    System,
    En,
    Zh,
}

/// Proof that a switch completed and its identity check passed; needed to set the active account.
///
/// Visibility cannot limit `issue` to the sibling `switching` module, so a source-scan test does.
#[derive(Debug)]
pub struct SwitchVerified(());

impl SwitchVerified {
    /// Only `switching` may call this, after the post-switch identity check has passed.
    pub fn issue() -> Self {
        Self(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Private: see [`SwitchVerified`]. Read through [`AppSettings::active_account_id`].
    active_account_id: Option<String>,
    pub dock_edge: DockEdge,
    pub display_id: Option<String>,
    pub vertical_offset: i32,
    pub launch_at_login: bool,
    pub always_on_top: bool,
    pub avoid_fullscreen: bool,
    active_refresh_seconds: u32,
    inactive_refresh_seconds: u32,
    pub reopen_codex_after_switch: bool,
    pub theme: Theme,
    pub reduce_motion: bool,
    /// Added in schema version 3; older documents default to `System`.
    #[serde(default = "follow_the_system")]
    pub language: Language,
    /// Added in schema version 4; older documents default to the bar.
    #[serde(default = "the_bar")]
    pub dock_shape: DockShape,
    /// Added in schema version 4. While set, the window is not shown and the tray is the only
    /// way back; it persists across restarts.
    #[serde(default)]
    pub dock_hidden: bool,
}

fn the_bar() -> DockShape {
    DockShape::Bar
}

fn follow_the_system() -> Language {
    Language::System
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_account_id: None,
            dock_edge: DockEdge::Right,
            display_id: None,
            vertical_offset: 0,
            launch_at_login: false,
            always_on_top: true,
            avoid_fullscreen: true,
            active_refresh_seconds: DEFAULT_ACTIVE_REFRESH_SECONDS,
            inactive_refresh_seconds: DEFAULT_INACTIVE_REFRESH_SECONDS,
            reopen_codex_after_switch: true,
            theme: Theme::System,
            reduce_motion: false,
            language: Language::System,
            dock_shape: DockShape::Bar,
            dock_hidden: false,
        }
    }
}

impl AppSettings {
    pub fn active_account_id(&self) -> Option<&str> {
        self.active_account_id.as_deref()
    }

    /// Records which account is now active; requires a [`SwitchVerified`].
    pub fn set_active_account_id(&mut self, id: Option<String>, _verified: &SwitchVerified) {
        self.active_account_id = id;
    }

    pub fn active_refresh_seconds(&self) -> u32 {
        self.active_refresh_seconds
    }

    pub fn inactive_refresh_seconds(&self) -> u32 {
        self.inactive_refresh_seconds
    }

    /// Resets out-of-range intervals to the defaults and returns the corrected field names.
    pub fn normalise(&mut self) -> Vec<&'static str> {
        let mut corrected = Vec::new();
        if !is_valid_interval(self.active_refresh_seconds) {
            self.active_refresh_seconds = DEFAULT_ACTIVE_REFRESH_SECONDS;
            corrected.push("activeRefreshSeconds");
        }
        if !is_valid_interval(self.inactive_refresh_seconds) {
            self.inactive_refresh_seconds = DEFAULT_INACTIVE_REFRESH_SECONDS;
            corrected.push("inactiveRefreshSeconds");
        }
        corrected
    }

    /// Sets the refresh intervals, rejecting values outside the allowed range.
    pub fn set_refresh_seconds(&mut self, active: u32, inactive: u32) -> Result<(), &'static str> {
        if !is_valid_interval(active) || !is_valid_interval(inactive) {
            return Err("refresh interval must be between 30 and 3600 seconds");
        }
        self.active_refresh_seconds = active;
        self.inactive_refresh_seconds = inactive;
        Ok(())
    }
}

fn is_valid_interval(seconds: u32) -> bool {
    (MIN_REFRESH_SECONDS..=MAX_REFRESH_SECONDS).contains(&seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_product_specification() {
        let settings = AppSettings::default();

        assert_eq!(settings.active_account_id(), None);
        assert_eq!(settings.dock_edge, DockEdge::Right);
        assert_eq!(settings.display_id, None);
        assert_eq!(settings.vertical_offset, 0);
        assert!(!settings.launch_at_login);
        assert!(settings.always_on_top);
        assert!(settings.avoid_fullscreen);
        assert_eq!(settings.active_refresh_seconds(), 60);
        assert_eq!(settings.inactive_refresh_seconds(), 300);
        assert!(settings.reopen_codex_after_switch);
        assert_eq!(settings.theme, Theme::System);
        assert!(!settings.reduce_motion);
        assert_eq!(settings.language, Language::System);
        assert_eq!(settings.dock_shape, DockShape::Bar);
        assert!(!settings.dock_hidden);
    }

    /// Settings written before the language existed must follow the operating system.
    #[test]
    fn settings_without_a_language_follow_the_system() {
        let without: AppSettings = serde_json::from_str(
            r#"{"activeAccountId":null,"dockEdge":"right","displayId":null,"verticalOffset":0,
                "launchAtLogin":false,"alwaysOnTop":true,"avoidFullscreen":true,
                "activeRefreshSeconds":60,"inactiveRefreshSeconds":300,
                "reopenCodexAfterSwitch":true,"theme":"system","reduceMotion":false}"#,
        )
        .expect("a version-2 settings block still parses");

        assert_eq!(without.language, Language::System);
    }

    #[test]
    fn an_out_of_range_interval_falls_back_and_is_reported() {
        let mut settings = AppSettings {
            active_refresh_seconds: 1,
            inactive_refresh_seconds: 99_999,
            ..AppSettings::default()
        };

        let corrected = settings.normalise();

        assert_eq!(
            corrected,
            vec!["activeRefreshSeconds", "inactiveRefreshSeconds"],
            "a corrected value must be reported, not silently accepted"
        );
        assert_eq!(settings.active_refresh_seconds(), 60);
        assert_eq!(settings.inactive_refresh_seconds(), 300);
    }

    #[test]
    fn a_valid_interval_is_left_alone() {
        let mut settings = AppSettings::default();
        settings
            .set_refresh_seconds(30, 3600)
            .expect("both bounds are valid");

        assert!(settings.normalise().is_empty());
        assert_eq!(settings.active_refresh_seconds(), 30);
        assert_eq!(settings.inactive_refresh_seconds(), 3600);
    }

    #[test]
    fn an_interval_just_outside_the_bounds_is_rejected() {
        let mut settings = AppSettings::default();

        assert!(settings.set_refresh_seconds(29, 300).is_err());
        assert!(settings.set_refresh_seconds(60, 3601).is_err());
        // The rejected call must not have changed anything.
        assert_eq!(settings.active_refresh_seconds(), 60);
    }

    #[test]
    fn the_active_account_survives_a_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.set_active_account_id(Some("acct-1".to_owned()), &SwitchVerified::issue());

        let json = serde_json::to_string(&settings).expect("serialises");
        let restored: AppSettings = serde_json::from_str(&json).expect("deserialises");

        assert_eq!(restored.active_account_id(), Some("acct-1"));
        assert!(
            json.contains("activeAccountId"),
            "wire form stays camelCase"
        );
    }

    /// A source scan, because visibility cannot be restricted to a sibling module.
    #[test]
    fn only_switching_may_issue_a_switch_verified_token() {
        const ALLOWED: [&str; 2] = ["switching", "settings.rs"];

        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        visit(&source, &mut |path: &std::path::Path| {
            let text = std::fs::read_to_string(path).expect("source is readable");
            // Only production code is in scope; tests may build the token to exercise the setter.
            let production = text.split("#[cfg(test)]").next().unwrap_or_default();
            if !production.contains("SwitchVerified::issue(") {
                return;
            }
            let as_string = path.to_string_lossy().replace(char::from(92u8), "/");
            if !ALLOWED.iter().any(|allowed| as_string.contains(allowed)) {
                offenders.push(as_string);
            }
        });

        assert!(
            offenders.is_empty(),
            "only `switching` may issue a SwitchVerified token; found: {offenders:?}"
        );
    }

    fn visit(directory: &std::path::Path, action: &mut impl FnMut(&std::path::Path)) {
        for entry in std::fs::read_dir(directory).expect("the source tree is readable") {
            let path = entry.expect("entry is readable").path();
            if path.is_dir() {
                visit(&path, action);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                action(&path);
            }
        }
    }
}
