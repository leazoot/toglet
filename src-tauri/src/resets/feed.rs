//! The reset feed's `GET /api/v1/status` body, read into a domain shape.
//!
//! Strict where the contract is strict (required fields), lenient where it is lenient (unknown
//! fields are ignored, `null` stays `None`). Nothing is guessed: a body that does not match the
//! feed's own OpenAPI document is refused as a whole rather than half-read.

use serde::{Deserialize, Serialize};

use crate::autorun::parse_rfc3339;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::text::one_line;

const PHASE: Phase = Phase::Resets;

/// The longest third-party sentence that may leave Rust, in characters. It is shown in a
/// tooltip, so it has to be readable, and never in a notification or a log.
pub const TEXT_CHARS: usize = 240;

/// Whether a reset applied to everyone or granted a banked credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetKind {
    Regular,
    Banked,
}

impl ResetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::Banked => "banked",
        }
    }
}

/// How sure the feed's classifier is about a forecast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchLevel {
    Elevated,
    Strong,
}

impl WatchLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Elevated => "elevated",
            Self::Strong => "strong",
        }
    }
}

/// A reset that was announced or observed. `text` is the announcement, already one line and
/// capped; `None` when it was blank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reset {
    pub id: String,
    pub kind: ResetKind,
    /// Unix seconds.
    pub announced_at: i64,
    pub text: Option<String>,
}

/// An announced reset still waiting for evidence that it happened. A `scheduled_for` in the
/// past does not mean it did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scheduled {
    pub id: String,
    pub kind: ResetKind,
    pub announced_at: i64,
    pub scheduled_for: Option<i64>,
    pub text: Option<String>,
}

/// An AI-classified forecast. The feed's own words: not an official OpenAI commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub level: WatchLevel,
    /// `None` when the classifier gave no figure; never shown as 0.
    pub chance_percent: Option<u8>,
    pub forecast_window: String,
    pub observed_at: i64,
    pub expires_at: i64,
    pub text: Option<String>,
}

/// Aggregate figures. Every optional one is unknown when `None`, never zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: u64,
    pub last_reset_at: Option<i64>,
    pub days_since_last: Option<f64>,
    pub avg_interval_days: Option<f64>,
}

/// One reading of the feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetStatus {
    pub latest_reset: Option<Reset>,
    pub scheduled_reset: Option<Scheduled>,
    pub active_watch: Option<Watch>,
    pub stats: Stats,
    /// When the feed generated this body, unix seconds.
    pub generated_at: i64,
}

// The wire shapes, kept private: everything the interface sees goes through the domain types.

#[derive(Deserialize)]
struct Body {
    data: Data,
    meta: Meta,
}

#[derive(Deserialize)]
struct Data {
    latest_reset: Option<ResetDto>,
    scheduled_reset: Option<ScheduledDto>,
    active_watch: Option<WatchDto>,
    stats: StatsDto,
}

#[derive(Deserialize)]
struct Meta {
    generated_at: String,
}

#[derive(Deserialize)]
struct ResetDto {
    id: String,
    reset_type: ResetKind,
    announced_at: String,
    text: String,
}

#[derive(Deserialize)]
struct ScheduledDto {
    id: String,
    reset_type: ResetKind,
    announced_at: String,
    scheduled_for: Option<String>,
    text: String,
}

#[derive(Deserialize)]
struct WatchDto {
    level: WatchLevel,
    reset_chance_percent: Option<u8>,
    forecast_window: String,
    observed_at: String,
    expires_at: String,
    text: String,
}

#[derive(Deserialize)]
struct StatsDto {
    total: u64,
    last_reset_at: Option<String>,
    days_since_last: Option<f64>,
    avg_interval_days: Option<f64>,
}

/// Reads a `200` body. Any departure from the contract refuses the whole body.
pub fn parse(body: &str) -> Result<ResetStatus> {
    let parsed: Body =
        serde_json::from_str(body).map_err(|_| unreadable("not the documented shape"))?;
    Ok(ResetStatus {
        latest_reset: parsed.data.latest_reset.map(reset).transpose()?,
        scheduled_reset: parsed.data.scheduled_reset.map(scheduled).transpose()?,
        active_watch: parsed.data.active_watch.map(watch).transpose()?,
        stats: Stats {
            total: parsed.data.stats.total,
            last_reset_at: parsed
                .data
                .stats
                .last_reset_at
                .as_deref()
                .map(instant)
                .transpose()?,
            days_since_last: parsed
                .data
                .stats
                .days_since_last
                .filter(|days| days.is_finite()),
            avg_interval_days: parsed
                .data
                .stats
                .avg_interval_days
                .filter(|days| days.is_finite()),
        },
        generated_at: instant(&parsed.meta.generated_at)?,
    })
}

fn reset(dto: ResetDto) -> Result<Reset> {
    Ok(Reset {
        id: identifier(dto.id)?,
        kind: dto.reset_type,
        announced_at: instant(&dto.announced_at)?,
        text: one_line(&dto.text, TEXT_CHARS),
    })
}

fn scheduled(dto: ScheduledDto) -> Result<Scheduled> {
    Ok(Scheduled {
        id: identifier(dto.id)?,
        kind: dto.reset_type,
        announced_at: instant(&dto.announced_at)?,
        scheduled_for: dto.scheduled_for.as_deref().map(instant).transpose()?,
        text: one_line(&dto.text, TEXT_CHARS),
    })
}

fn watch(dto: WatchDto) -> Result<Watch> {
    Ok(Watch {
        level: dto.level,
        chance_percent: dto.reset_chance_percent.filter(|percent| *percent <= 100),
        forecast_window: one_line(&dto.forecast_window, TEXT_CHARS).unwrap_or_default(),
        observed_at: instant(&dto.observed_at)?,
        expires_at: instant(&dto.expires_at)?,
        text: one_line(&dto.text, TEXT_CHARS),
    })
}

/// The contract bounds ids to 1..=64 characters; an id is a dedupe key, so an empty or
/// oversized one would break dedupe rather than merely look odd.
fn identifier(id: String) -> Result<String> {
    let length = id.chars().count();
    if (1..=64).contains(&length) && !id.chars().any(char::is_control) {
        Ok(id)
    } else {
        Err(unreadable("an id outside the documented bounds"))
    }
}

fn instant(text: &str) -> Result<i64> {
    parse_rfc3339(text).ok_or_else(|| unreadable("a timestamp that is not RFC 3339"))
}

fn unreadable(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::ResetFeedUnreadable,
        PHASE,
        false,
        UserAction::None,
    )
    .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from the live feed on 2026-09-17.
    const CAPTURED: &str = r#"{"data":{"latest_reset":{"id":"2098685367058612394","reset_type":"regular","announced_at":"2026-09-12T08:09:17.000Z","text":"Reset all propagated. Sweet dreams. https://t.co/VgKVUixoJG","source":{"type":"x_post","author":"thsottiaux","url":"https://x.com/thsottiaux/status/2098685367058612394"}},"scheduled_reset":null,"active_watch":null,"stats":{"total":53,"last_reset_at":"2026-09-12T08:09:17.000Z","days_since_last":4.9,"avg_interval_days":6.9}},"meta":{"api_version":"v1","generated_at":"2026-09-17T06:25:54.627Z"}}"#;

    #[test]
    fn the_captured_body_reads_into_the_domain_shape() {
        let status = parse(CAPTURED).expect("the live body parses");
        let latest = status.latest_reset.expect("a latest reset");
        assert_eq!(latest.id, "2098685367058612394");
        assert_eq!(latest.kind, ResetKind::Regular);
        assert_eq!(latest.announced_at, 1_789_200_557);
        assert_eq!(
            latest.text.as_deref(),
            Some("Reset all propagated. Sweet dreams. https://t.co/VgKVUixoJG")
        );
        assert!(status.scheduled_reset.is_none());
        assert!(status.active_watch.is_none());
        assert_eq!(status.stats.total, 53);
        assert_eq!(status.stats.days_since_last, Some(4.9));
        assert_eq!(status.stats.avg_interval_days, Some(6.9));
        assert_eq!(status.generated_at, 1_789_626_354);
    }

    #[test]
    fn a_null_figure_stays_unknown_rather_than_becoming_zero() {
        let body = CAPTURED
            .replace(r#""days_since_last":4.9"#, r#""days_since_last":null"#)
            .replace(r#""avg_interval_days":6.9"#, r#""avg_interval_days":null"#)
            .replace(
                r#""active_watch":null"#,
                r#""active_watch":{"level":"strong","reset_chance_percent":null,"forecast_window":"next 48h","observed_at":"2026-09-17T01:00:00Z","expires_at":"2026-09-19T01:00:00Z","text":"Signals point to a reset.","source":{"type":"observed"}}"#,
            );
        let status = parse(&body).expect("parses");
        assert_eq!(status.stats.days_since_last, None);
        assert_eq!(status.stats.avg_interval_days, None);
        let watch = status.active_watch.expect("a watch");
        assert_eq!(watch.level, WatchLevel::Strong);
        assert_eq!(watch.chance_percent, None);
        assert_eq!(watch.forecast_window, "next 48h");
    }

    #[test]
    fn a_scheduled_reset_keeps_a_missing_time_as_unknown() {
        let body = CAPTURED.replace(
            r#""scheduled_reset":null"#,
            r#""scheduled_reset":{"id":"s1","status":"scheduled","reset_type":"banked","announced_at":"2026-09-17T02:00:00Z","scheduled_for":null,"text":"  One banked   reset\ntomorrow. ","source":{"type":"observed"}}"#,
        );
        let status = parse(&body).expect("parses");
        let scheduled = status.scheduled_reset.expect("scheduled");
        assert_eq!(scheduled.kind, ResetKind::Banked);
        assert_eq!(scheduled.scheduled_for, None);
        assert_eq!(
            scheduled.text.as_deref(),
            Some("One banked reset tomorrow.")
        );
    }

    #[test]
    fn unknown_fields_are_ignored_and_a_missing_required_one_refuses_the_body() {
        let extra = CAPTURED.replace(r#""meta":{"#, r#""meta":{"colour":"teal","#);
        assert!(parse(&extra).is_ok());

        let missing = CAPTURED.replace(r#""total":53,"#, "");
        let error = parse(&missing).expect_err("refused");
        assert_eq!(error.code(), ErrorCode::ResetFeedUnreadable);
        assert!(!error.retryable());
    }

    #[test]
    fn a_body_that_is_not_json_is_refused_without_quoting_it() {
        let error = parse("<html>proxy login page for secret-host</html>").expect_err("refused");
        assert_eq!(error.code(), ErrorCode::ResetFeedUnreadable);
        assert!(!format!("{error:?}").contains("secret-host"));
    }

    #[test]
    fn a_timestamp_that_is_not_rfc3339_refuses_the_body() {
        let body = CAPTURED.replace("2026-09-17T06:25:54.627Z", "yesterday");
        assert_eq!(
            parse(&body).expect_err("refused").code(),
            ErrorCode::ResetFeedUnreadable
        );
    }

    #[test]
    fn a_long_announcement_is_cut_before_it_can_leave() {
        let long = "word ".repeat(200);
        let body = CAPTURED.replace(
            "Reset all propagated. Sweet dreams. https://t.co/VgKVUixoJG",
            &long,
        );
        let text = parse(&body)
            .expect("parses")
            .latest_reset
            .expect("latest")
            .text
            .expect("text");
        assert_eq!(text.chars().count(), TEXT_CHARS + 1);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn an_empty_id_is_refused_because_it_would_break_dedupe() {
        let body = CAPTURED.replace(r#""id":"2098685367058612394""#, r#""id":"""#);
        assert_eq!(
            parse(&body).expect_err("refused").code(),
            ErrorCode::ResetFeedUnreadable
        );
    }
}
