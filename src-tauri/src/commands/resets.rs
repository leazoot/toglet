//! Reset-alert settings and state. Nothing here can reach the feed: the poll thread does the
//! reading, and this file only holds what it found and what the user chose.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use super::notify::Notifications;
use super::views::ErrorView;
use crate::diagnostics::{Level, LogRecord, Phase, Result, log};
use crate::resets::{Announcement, ResetStatus, ResetsConfig, ResetsStore, SITE_URL};
use crate::storage::LoadOutcome;

const PHASE: Phase = Phase::Resets;

/// Pushed after every reading and every save, with the whole view.
pub const RESETS_STATE_EVENT: &str = "resets://state";

/// Pushed once per event worth a notification; the interface turns it into words.
pub const RESET_ANNOUNCED_EVENT: &str = "resets://announced";

/// A reading older than this is shown as stale: three polls without a fresh answer.
const STALE_AFTER_SECONDS: i64 = 900;

/// At most this many channels can be chosen; the channel list itself is capped at the same.
const MAX_CHOSEN: usize = crate::notify::MAX_CHANNELS;

/// The settings and the last reading, as the interface sees them. The reading is the domain
/// type itself: it holds nothing but public feed data, already cut to size.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetsView {
    pub enabled: bool,
    pub channel_ids: Vec<String>,
    pub status: Option<ResetStatus>,
    /// Unix seconds of the reading `status` came from; `None` when there has been none.
    pub fetched_at: Option<i64>,
    /// True when `status` is older than three polls. Never true without a `status`.
    pub stale: bool,
    /// Stable code of the last failed reading, cleared by the next good one.
    pub last_error: Option<String>,
}

/// One event, tagged by kind, with only codes and timestamps: the sentence is the interface's.
// `rename_all` on the enum names the variants (`kind`); each variant needs its own for its
// fields, which serde does not inherit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AnnouncementView {
    #[serde(rename_all = "camelCase")]
    Reset { reset_type: &'static str, at: i64 },
    #[serde(rename_all = "camelCase")]
    Scheduled {
        reset_type: &'static str,
        at: i64,
        scheduled_for: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    Watch {
        level: &'static str,
        chance_percent: Option<u8>,
        at: i64,
    },
}

impl AnnouncementView {
    pub fn of(announcement: &Announcement) -> Self {
        match announcement {
            Announcement::Reset { kind, at } => Self::Reset {
                reset_type: kind.as_str(),
                at: *at,
            },
            Announcement::Scheduled {
                kind,
                at,
                scheduled_for,
            } => Self::Scheduled {
                reset_type: kind.as_str(),
                at: *at,
                scheduled_for: *scheduled_for,
            },
            Announcement::Watch {
                level,
                chance_percent,
                at,
            } => Self::Watch {
                level: level.as_str(),
                chance_percent: *chance_percent,
                at: *at,
            },
        }
    }
}

/// What the page sends when the user saves.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetsDraft {
    pub enabled: bool,
    pub channel_ids: Vec<String>,
}

/// The poll thread's working state, kept beside the settings so a save can see the latest
/// reading and a reading can see the latest settings.
#[derive(Debug, Default)]
struct Live {
    /// The last failed reading's code, so the banner can say why nothing is fresh.
    last_error: Option<&'static str>,
    /// The entity tag of the last fresh body; memory only, a restart just fetches once.
    etag: Option<String>,
    /// When the feed last confirmed the cached reading, fresh or unchanged. Memory only: the
    /// file's `fetched_at` moves only on a fresh body, so a `304` does not cost a write.
    confirmed_at: Option<i64>,
}

pub struct Resets {
    store: ResetsStore,
    config: Mutex<ResetsConfig>,
    live: Mutex<Live>,
    /// Nudges the poll thread out of its wait, so a switch turned on reads at once.
    wake: Sender<()>,
}

impl Resets {
    /// Loads the settings. An unusable file is replaced by defaults, which have the feature off.
    /// The returned receiver belongs to the poll thread.
    pub fn load(data_directory: &std::path::Path) -> (Self, Receiver<()>) {
        let store = ResetsStore::new(data_directory);
        let (config, outcome) = store.load();
        if let LoadOutcome::Rebuilt { .. } = outcome {
            log(&LogRecord::new(Level::Warn, "resets_settings_rebuilt_at_start").with_phase(PHASE));
        }
        let (wake, woken) = channel();
        (
            Self {
                store,
                config: Mutex::new(config),
                live: Mutex::new(Live::default()),
                wake,
            },
            woken,
        )
    }

    fn config(&self) -> std::sync::MutexGuard<'_, ResetsConfig> {
        // Poisoning is harmless: the guarded value is a plain settings record.
        self.config
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn snapshot(&self) -> ResetsConfig {
        self.config().clone()
    }

    /// The tag to send with the next request, if a fresh body has been seen this run.
    pub fn etag(&self) -> Option<String> {
        self.live().etag.clone()
    }

    /// Records a fresh reading: markers, cache and time go to the file; the tag stays in memory.
    pub fn record_reading(&self, status: ResetStatus, etag: Option<String>, now: i64) {
        let mut config = self.config();
        let (markers, _) = crate::resets::announce(&config.markers, &status);
        config.markers = markers;
        config.cached = Some(status);
        config.fetched_at = Some(now);
        if let Err(error) = self.store.save(&config) {
            log(&LogRecord::from_error(
                "resets_settings_not_written",
                &error,
            ));
        }
        drop(config);
        let mut live = self.live();
        live.etag = etag;
        live.last_error = None;
        live.confirmed_at = Some(now);
    }

    /// Records a `304`: the cached reading is current as of now. Nothing is written.
    pub fn record_unchanged(&self, now: i64) {
        let mut live = self.live();
        live.last_error = None;
        live.confirmed_at = Some(now);
    }

    /// Records a failed reading by its code only.
    pub fn record_failure(&self, code: &'static str) {
        self.live().last_error = Some(code);
    }

    /// The events a reading would announce, decided against the stored markers. Called before
    /// [`record_reading`](Self::record_reading) moves the markers on.
    pub fn announcements(&self, status: &ResetStatus) -> Vec<Announcement> {
        let config = self.config();
        crate::resets::announce(&config.markers, status).1
    }

    /// `existing` is the channel list as it stands: an id whose channel was removed since it was
    /// chosen is left out here, so it is neither counted nor sent to. The file is only cleaned
    /// on the next save; reading is where the truth is enforced.
    pub fn view(&self, now: i64, existing: &[String]) -> ResetsView {
        let config = self.config();
        let live = self.live();
        let fetched_at = live.confirmed_at.or(config.fetched_at);
        let stale =
            config.cached.is_some() && fetched_at.is_none_or(|at| now - at > STALE_AFTER_SECONDS);
        ResetsView {
            enabled: config.enabled,
            channel_ids: config
                .channel_ids
                .iter()
                .filter(|id| existing.contains(id))
                .cloned()
                .collect(),
            status: config.cached.clone(),
            fetched_at,
            stale,
            last_error: live.last_error.map(str::to_owned),
        }
    }

    /// Tells everyone what the state is now. Best effort: a missed event is caught by the next.
    pub fn announce_state(&self, app: &AppHandle) {
        let existing = app.state::<Notifications>().channel_ids();
        if app
            .emit(RESETS_STATE_EVENT, self.view(unix_seconds(), &existing))
            .is_err()
        {
            log(&LogRecord::new(Level::Warn, "resets_state_not_delivered").with_phase(PHASE));
        }
    }
}

/// Unix seconds now; the clock is not injected here because nothing below depends on it.
pub fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[tauri::command]
pub fn read_resets(
    resets: State<'_, Resets>,
    notifications: State<'_, Notifications>,
) -> ResetsView {
    resets.view(unix_seconds(), &notifications.channel_ids())
}

/// Saves the switch and the chosen channels. Switching on wakes the poll thread so the banner
/// fills within seconds; switching off forgets the markers so a later switch-on starts quiet.
#[tauri::command]
pub fn save_resets(
    app: AppHandle,
    resets: State<'_, Resets>,
    notifications: State<'_, Notifications>,
    draft: ResetsDraft,
) -> std::result::Result<ResetsView, ErrorView> {
    reported(
        "resets_save_failed",
        save(&app, &resets, &notifications, draft),
    )
}

fn save(
    app: &AppHandle,
    resets: &Resets,
    notifications: &Notifications,
    draft: ResetsDraft,
) -> Result<ResetsView> {
    let chosen = chosen_channels(draft.channel_ids, &notifications.channel_ids());

    let mut config = resets.config();
    let was_enabled = config.enabled;
    if draft.enabled {
        config.enabled = true;
    } else {
        config.disable();
    }
    config.channel_ids = chosen;
    resets.store.save(&config)?;
    drop(config);

    if draft.enabled && !was_enabled && resets.wake.send(()).is_err() {
        // The receiver is gone only if the poll thread never started or died; the next
        // start will read, so this is worth a line in the log and nothing more.
        log(&LogRecord::new(Level::Warn, "resets_poll_not_running").with_phase(PHASE));
    }
    resets.announce_state(app);
    Ok(resets.view(unix_seconds(), &notifications.channel_ids()))
}

/// Keeps ids that name a channel that exists, each once, in the order given, capped.
fn chosen_channels(requested: Vec<String>, existing: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for id in requested {
        if existing.contains(&id) && !kept.contains(&id) {
            kept.push(id);
        }
        if kept.len() == MAX_CHOSEN {
            break;
        }
    }
    kept
}

/// Opens the feed's site in the browser: the credit its terms ask for. A constant address,
/// through the same `https`-only door the sign-in flow uses.
#[tauri::command]
pub fn open_resets_site() -> std::result::Result<(), ErrorView> {
    reported(
        "resets_site_not_opened",
        crate::process::open_url(SITE_URL, PHASE),
    )
}

fn reported<T>(event: &'static str, outcome: Result<T>) -> std::result::Result<T, ErrorView> {
    outcome.map_err(|error| {
        log(&LogRecord::from_error(event, &error));
        ErrorView::from(error)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn only_channels_that_exist_are_kept_each_once_in_order() {
        let kept = chosen_channels(
            ids(&["chan-b", "chan-zz", "chan-a", "chan-b"]),
            &ids(&["chan-a", "chan-b"]),
        );
        assert_eq!(kept, ids(&["chan-b", "chan-a"]));
    }

    #[test]
    fn the_choice_is_capped_at_the_channel_limit() {
        let many: Vec<String> = (0..20).map(|n| format!("chan-{n}")).collect();
        let kept = chosen_channels(many.clone(), &many);
        assert_eq!(kept.len(), MAX_CHOSEN);
    }

    #[test]
    fn an_announcement_serialises_as_a_tagged_record_of_codes() {
        let view = AnnouncementView::of(&Announcement::Watch {
            level: crate::resets::WatchLevel::Strong,
            chance_percent: None,
            at: 7,
        });
        let json = serde_json::to_string(&view).expect("serialises");
        assert_eq!(
            json,
            r#"{"kind":"watch","level":"strong","chancePercent":null,"at":7}"#
        );
    }

    /// The page draws a switch per channel that exists, so a stale id would be invisible there
    /// while the row's count and the fan-out still used it - which is how this was found.
    #[test]
    fn a_channel_removed_since_it_was_chosen_is_neither_counted_nor_kept() {
        let home = crate::codex_home::IsolatedHome::create(Phase::Storage).expect("scratch");
        let (resets, _woken) = Resets::load(home.path());
        resets.config().channel_ids = ids(&["chan-a", "chan-gone", "chan-b"]);

        let view = resets.view(0, &ids(&["chan-a", "chan-b"]));

        assert_eq!(view.channel_ids, ids(&["chan-a", "chan-b"]));
    }

    #[test]
    fn the_view_names_nothing_but_the_feed_and_the_choice() {
        let view = ResetsView {
            enabled: true,
            channel_ids: ids(&["chan-1"]),
            status: None,
            fetched_at: None,
            stale: false,
            last_error: Some("network_unavailable".to_owned()),
        };
        let json = serde_json::to_string(&view).expect("serialises");
        assert!(json.contains(r#""channelIds":["chan-1"]"#));
        assert!(json.contains(r#""lastError":"network_unavailable""#));
        assert!(!json.contains("https://"));
    }
}
