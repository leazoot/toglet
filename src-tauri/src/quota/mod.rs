//! Quota window normalisation, caching, refresh scheduling and backoff.
//!
//! Must not depend on `switching` or be able to write the default authentication. Window type
//! comes from the duration, never the `primary`/`secondary` slot; unknown values stay `None`.

mod cache;
mod normalize;
mod scheduler;

pub use cache::{QuotaSnapshot, QuotaSnapshotView, SOURCE_APP_SERVER, STALE_AFTER_SECONDS};
pub use normalize::{NormalisedQuota, QuotaWindow, ResetCredits, WindowKind, remaining_percent};
pub use scheduler::{
    BACKOFF_CAP_SECONDS, Backoff, EXPAND_REFRESH_AFTER_SECONDS, RefreshIntervals, RefreshState,
    RefreshTrigger, all_refreshable, due_now, due_on_expand,
};
