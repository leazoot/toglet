//! Quota window normalisation, caching, refresh scheduling and backoff.
//!
//! Depends on: `app_server`, `storage`, `diagnostics`. Must not depend on `switching`, and
//! must not be able to write the default authentication at all - enforced by module
//! visibility, not by convention.
//!
//! Hard constraints: the window type is decided by `windowDurationMins` (300 -> five hour,
//! 10080 -> weekly), never by `primary`/`secondary` position; unknown values stay `None` and
//! are never collapsed to `0`.
//!
//! Implemented so far: window normalisation, snapshot caching with derived staleness and
//! refresh scheduling with backoff.

mod cache;
mod normalize;
mod scheduler;

pub use cache::{QuotaSnapshot, QuotaSnapshotView, SOURCE_APP_SERVER, STALE_AFTER_SECONDS};
pub use normalize::{NormalisedQuota, QuotaWindow, WindowKind, remaining_percent};
pub use scheduler::{
    BACKOFF_CAP_SECONDS, Backoff, EXPAND_REFRESH_AFTER_SECONDS, RefreshIntervals, RefreshState,
    RefreshTrigger, all_refreshable, due_now, due_on_expand,
};
