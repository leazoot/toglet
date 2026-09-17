//! One of the only three files that open outbound connections (with `notify::send` and
//! `remote::poll`), and the only one whose destination is not an address the user typed.
//!
//! A single conditional `GET` of a public, keyless feed. Nothing about this machine's accounts
//! rides along: no cookie, no token, no e-mail, no quota. The reply is third-party text and is
//! handed to `feed::parse` untouched; it is never logged or quoted in an error.

use std::sync::OnceLock;
use std::time::Duration;

use super::feed::{self, ResetStatus};
use crate::diagnostics::{ErrorCode, Phase, TogletError, UserAction};
use crate::quota::Backoff;

const PHASE: Phase = Phase::Resets;

/// The feed. A compile-time constant: the user cannot point this at another host.
pub const STATUS_URL: &str = "https://codex-resets.com/api/v1/status";

/// The site the feed's terms ask to be linked back to wherever its data is shown.
pub const SITE_URL: &str = "https://codex-resets.com";

/// Interval between conditional requests while the feature is on.
///
/// The feed's edge cache revalidates once a minute (`s-maxage=60`), so anything faster only
/// reads the same cached body; resets come about weekly, so a few minutes' delay is nothing.
pub const POLL: Duration = Duration::from_secs(300);

/// A feed that asks to be left alone gets at most this long, whatever `Retry-After` says.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(3600);

/// The whole request; the body is 500 bytes, so this is a network problem, not a slow feed.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Longest entity tag echoed back; the feed's are 66 characters.
const ETAG_CHARS: usize = 256;

/// What one request brought back.
#[derive(Debug)]
pub enum Fetched {
    /// `304`: the body has not changed since the tag it was given.
    Unchanged,
    /// A new body. Boxed: the reading is a few hundred bytes wide and `Unchanged` is nothing,
    /// and this value is passed around, not stored.
    Fresh {
        status: Box<ResetStatus>,
        etag: Option<String>,
    },
}

/// A failed request, and how long the feed asked to be left alone, if it said.
#[derive(Debug)]
pub struct Retry {
    pub error: TogletError,
    pub after: Option<Duration>,
}

/// How long to wait before the next request.
pub fn next_wait(backoff: Backoff, retry_after: Option<Duration>) -> Duration {
    if let Some(asked) = retry_after {
        return asked.min(RETRY_AFTER_CAP);
    }
    if backoff.failures() > 0 {
        return backoff.delay();
    }
    POLL
}

/// Reads the feed once. `etag` is the tag from the last fresh body, so an unchanged feed
/// costs a `304` and no body.
pub async fn fetch(etag: Option<&str>) -> Result<Fetched, Retry> {
    // Re-checked at the moment of use even though the address is a constant: the check is the
    // same one every outbound file runs, and a constant that fails it should never be sent to.
    if !crate::net::is_safe_endpoint(STATUS_URL) {
        return Err(plain(unreadable(
            "the feed address is not one Toglet may read",
        )));
    }

    let mut request = client()
        .map_err(plain)?
        .get(STATUS_URL)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(tag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, tag);
    }
    let response = request
        .send()
        .await
        .map_err(|error| plain(unreachable(&error.to_string())))?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::Unchanged);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(Retry {
            error: unreachable("the feed asked to be left alone for a while"),
            after: retry_after(&response),
        });
    }
    if status.is_server_error() {
        return Err(plain(unreachable("the feed reported a failure of its own")));
    }
    if !status.is_success() {
        // A 4xx other than 429 means the address no longer means what this build expects.
        return Err(plain(unreadable("the feed did not answer as documented")));
    }

    let tag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.chars().count() <= ETAG_CHARS && value.is_ascii())
        .map(str::to_owned);
    // Untrusted text: parsed, never logged.
    let body = response.text().await.unwrap_or_default();
    let status = feed::parse(&body).map_err(plain)?;
    Ok(Fetched::Fresh {
        status: Box::new(status),
        etag: tag,
    })
}

/// `Retry-After` in seconds; the HTTP-date form is ignored rather than guessed at.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Shared client. Redirects are refused: the address is a constant and must stay one.
fn client() -> Result<&'static reqwest::Client, TogletError> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(TIMEOUT)
                .connect_timeout(TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("Toglet/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| {
            TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
                .with_detail("the outbound client could not be prepared")
        })
}

fn plain(error: TogletError) -> Retry {
    Retry { error, after: None }
}

/// Feed unreachable or failing on its own; retryable, nothing to fix here.
fn unreachable(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::NetworkUnavailable,
        PHASE,
        true,
        UserAction::CheckNetwork,
    )
    .with_detail(detail)
}

/// The feed answered with something this build cannot read; not retryable.
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

    #[test]
    fn the_feed_address_is_a_plain_https_constant_that_passes_the_shared_check() {
        assert!(STATUS_URL.starts_with("https://codex-resets.com/"));
        assert!(crate::net::is_safe_endpoint(STATUS_URL));
        assert_eq!(crate::net::host_of(STATUS_URL), Some("codex-resets.com"));
        assert_eq!(crate::net::host_of(SITE_URL), Some("codex-resets.com"));
    }

    #[test]
    fn the_site_link_is_something_the_browser_opener_accepts() {
        // `process::browser::open_url` takes only plain https addresses.
        assert!(SITE_URL.starts_with("https://"));
        assert!(
            !SITE_URL
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        );
    }

    #[test]
    fn a_healthy_feed_is_asked_every_five_minutes() {
        assert_eq!(next_wait(Backoff::new(), None), POLL);
        assert_eq!(POLL, Duration::from_secs(300));
    }

    #[test]
    fn a_feed_that_asks_to_be_left_alone_is_obeyed_up_to_an_hour() {
        assert_eq!(
            next_wait(Backoff::new(), Some(Duration::from_secs(90))),
            Duration::from_secs(90)
        );
        assert_eq!(
            next_wait(Backoff::new(), Some(Duration::from_secs(86_400))),
            RETRY_AFTER_CAP
        );
    }

    #[test]
    fn failures_back_off_instead_of_hammering() {
        let failed = Backoff::new().after_failure();
        assert_eq!(next_wait(failed, None), failed.delay());
        assert!(failed.delay() > Duration::ZERO);
    }

    #[test]
    fn errors_name_the_step_without_quoting_the_feed() {
        let error = unreachable("connection refused by secret-proxy");
        assert_eq!(error.code(), ErrorCode::NetworkUnavailable);
        assert!(error.retryable());
        let error = unreadable("x");
        assert_eq!(error.code(), ErrorCode::ResetFeedUnreadable);
        assert!(!error.retryable());
    }
}
