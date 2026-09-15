//! Reading and changing the settings the interface can offer.
//!
//! Only settings with working behaviour are exposed: a switch that changes nothing is worse
//! than none.

use serde::{Deserialize, Serialize};
use tauri::{State, WebviewWindow};

use super::state::AppState;
use super::views::ErrorView;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::storage::{AppSettings, DockEdge, DockShape, Language, Theme};
use crate::window;

/// The settings the interface may see. Not all of `AppSettings`: `activeAccountId` is not
/// user-editable, and `displayId` only records where the window was.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub dock_edge: &'static str,
    /// `bar` or `ring`. Rust sizes the hover target from the same value.
    pub dock_shape: &'static str,
    /// Bar centre in logical pixels below the work area's centre; the stylesheet and the hover
    /// target both use it.
    pub vertical_offset: i32,
    pub always_on_top: bool,
    pub active_refresh_seconds: u32,
    pub inactive_refresh_seconds: u32,
    pub reopen_codex_after_switch: bool,
    pub theme: &'static str,
    pub reduce_motion: bool,
    /// `system` until the user picks one; the interface resolves it from the webview's locale.
    pub language: &'static str,
}

impl SettingsView {
    pub(crate) fn of(settings: &AppSettings) -> Self {
        Self {
            dock_edge: edge_name(settings.dock_edge),
            dock_shape: shape_name(settings.dock_shape),
            vertical_offset: settings.vertical_offset,
            always_on_top: settings.always_on_top,
            active_refresh_seconds: settings.active_refresh_seconds(),
            inactive_refresh_seconds: settings.inactive_refresh_seconds(),
            reopen_codex_after_switch: settings.reopen_codex_after_switch,
            theme: theme_name(settings.theme),
            reduce_motion: settings.reduce_motion,
            language: language_name(settings.language),
        }
    }
}

/// A change to some of the settings. Every field is optional, and an absent one is left alone.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub dock_edge: Option<String>,
    pub dock_shape: Option<String>,
    pub always_on_top: Option<bool>,
    pub active_refresh_seconds: Option<u32>,
    pub inactive_refresh_seconds: Option<u32>,
    pub reopen_codex_after_switch: Option<bool>,
    pub theme: Option<String>,
    pub reduce_motion: Option<bool>,
    pub language: Option<String>,
}

#[tauri::command]
pub fn read_settings(state: State<'_, AppState>) -> SettingsView {
    state.read_document(|document| SettingsView::of(&document.settings))
}

/// Applies a change and returns the resulting settings, which the interface shows instead of
/// assuming its change took. Re-docks when the edge, shape or always-on-top changed, so the
/// hover target and clamped offset update immediately.
#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    window: WebviewWindow,
    patch: SettingsPatch,
) -> std::result::Result<SettingsView, ErrorView> {
    apply(state.inner(), &window, patch).map_err(ErrorView::from)
}

fn apply(state: &AppState, window: &WebviewWindow, patch: SettingsPatch) -> Result<SettingsView> {
    let edge = patch.dock_edge.as_deref().map(parse_edge).transpose()?;
    let shape = patch.dock_shape.as_deref().map(parse_shape).transpose()?;
    let theme = patch.theme.as_deref().map(parse_theme).transpose()?;
    let language = patch.language.as_deref().map(parse_language).transpose()?;

    let (view, moved) = state.with_document(|document| {
        let settings = &mut document.settings;
        let before = (
            settings.dock_edge,
            settings.dock_shape,
            settings.always_on_top,
        );

        if let Some(edge) = edge {
            settings.dock_edge = edge;
        }
        if let Some(shape) = shape {
            settings.dock_shape = shape;
        }
        if let Some(on_top) = patch.always_on_top {
            settings.always_on_top = on_top;
        }
        if let Some(theme) = theme {
            settings.theme = theme;
        }
        if let Some(language) = language {
            settings.language = language;
        }
        if let Some(reduce) = patch.reduce_motion {
            settings.reduce_motion = reduce;
        }
        if let Some(reopen) = patch.reopen_codex_after_switch {
            settings.reopen_codex_after_switch = reopen;
        }

        // Both intervals go through one setter, which validates them together and refuses the
        // whole change rather than accepting half of it.
        if patch.active_refresh_seconds.is_some() || patch.inactive_refresh_seconds.is_some() {
            let active = patch
                .active_refresh_seconds
                .unwrap_or_else(|| settings.active_refresh_seconds());
            let inactive = patch
                .inactive_refresh_seconds
                .unwrap_or_else(|| settings.inactive_refresh_seconds());
            settings
                .set_refresh_seconds(active, inactive)
                .map_err(|_| {
                    TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                        .with_detail("a refresh interval was outside the range settings allow")
                })?;
        }

        let moved = before
            != (
                settings.dock_edge,
                settings.dock_shape,
                settings.always_on_top,
            );
        Ok(((SettingsView::of(settings), moved), true))
    })?;

    if moved {
        let settings = state.read_document(|document| document.settings.clone());
        let outcome = window::dock_window(window, &settings)?;
        super::window::remember(state, &outcome)?;
        // Re-read: re-docking may have clamped the offset to the monitor.
        return Ok(state.read_document(|document| SettingsView::of(&document.settings)));
    }
    Ok(view)
}

fn parse_edge(value: &str) -> Result<DockEdge> {
    match value {
        "left" => Ok(DockEdge::Left),
        "right" => Ok(DockEdge::Right),
        _ => Err(rejected("dock edge")),
    }
}

fn parse_shape(value: &str) -> Result<DockShape> {
    match value {
        "bar" => Ok(DockShape::Bar),
        "ring" => Ok(DockShape::Ring),
        _ => Err(rejected("dock shape")),
    }
}

fn parse_theme(value: &str) -> Result<Theme> {
    match value {
        "system" => Ok(Theme::System),
        "dark" => Ok(Theme::Dark),
        "light" => Ok(Theme::Light),
        _ => Err(rejected("theme")),
    }
}

/// Accepts `system`, the fresh-install value, so a patch can return to following the OS.
fn parse_language(value: &str) -> Result<Language> {
    match value {
        "system" => Ok(Language::System),
        "en" => Ok(Language::En),
        "zh" => Ok(Language::Zh),
        _ => Err(rejected("language")),
    }
}

/// An unrecognised value is refused, never rounded to the nearest thing that parses.
fn rejected(what: &str) -> TogletError {
    TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
        .with_detail(&format!("an unrecognised {what} was rejected"))
}

fn edge_name(edge: DockEdge) -> &'static str {
    match edge {
        DockEdge::Left => "left",
        DockEdge::Right => "right",
    }
}

fn shape_name(shape: DockShape) -> &'static str {
    match shape {
        DockShape::Bar => "bar",
        DockShape::Ring => "ring",
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "system",
        Theme::Dark => "dark",
        Theme::Light => "light",
    }
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::System => "system",
        Language::En => "en",
        Language::Zh => "zh",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_side_it_does_not_recognise() {
        assert!(parse_edge("top").is_err());
    }

    #[test]
    fn refuses_a_shape_it_does_not_recognise() {
        assert!(parse_shape("island").is_err());
    }

    #[test]
    fn refuses_a_theme_it_does_not_recognise() {
        assert!(parse_theme("solarized").is_err());
    }

    #[test]
    fn refuses_a_language_it_does_not_recognise() {
        assert!(parse_language("fr").is_err());
        // Regional tags are resolved by the interface, not here.
        assert!(parse_language("zh-CN").is_err());
    }

    #[test]
    fn accepts_exactly_the_values_the_product_defines() {
        assert_eq!(parse_edge("left").expect("valid"), DockEdge::Left);
        assert_eq!(parse_edge("right").expect("valid"), DockEdge::Right);
        assert_eq!(parse_shape("bar").expect("valid"), DockShape::Bar);
        assert_eq!(parse_shape("ring").expect("valid"), DockShape::Ring);
        assert_eq!(parse_theme("system").expect("valid"), Theme::System);
        assert_eq!(parse_theme("dark").expect("valid"), Theme::Dark);
        assert_eq!(parse_theme("light").expect("valid"), Theme::Light);
        assert_eq!(parse_language("system").expect("valid"), Language::System);
        assert_eq!(parse_language("en").expect("valid"), Language::En);
        assert_eq!(parse_language("zh").expect("valid"), Language::Zh);
    }

    #[test]
    fn round_trips_every_name_it_produces() {
        // Every name a view emits must parse back through a patch.
        for edge in [DockEdge::Left, DockEdge::Right] {
            assert_eq!(parse_edge(edge_name(edge)).expect("valid"), edge);
        }
        for shape in [DockShape::Bar, DockShape::Ring] {
            assert_eq!(parse_shape(shape_name(shape)).expect("valid"), shape);
        }
        for theme in [Theme::System, Theme::Dark, Theme::Light] {
            assert_eq!(parse_theme(theme_name(theme)).expect("valid"), theme);
        }
        for language in [Language::System, Language::En, Language::Zh] {
            assert_eq!(
                parse_language(language_name(language)).expect("valid"),
                language
            );
        }
    }
}
