//! Reset alerts: the public reset feed the user can switch on, read into a status the panel
//! shows, with the moments worth a notification picked out.
//!
//! Off by default, and off means no request. Only `fetch` opens a connection, to a compile-time
//! address; the feed is a third party, so what comes back is treated as untrusted text that may
//! reach a tooltip and nothing else. This module cannot see credentials, accounts or the
//! continuation plan.

pub mod feed;
mod fetch;
mod store;
pub mod watch;

pub use feed::{Reset, ResetKind, ResetStatus, Scheduled, Stats, Watch, WatchLevel};
pub use fetch::{Fetched, POLL, Retry, SITE_URL, STATUS_URL, fetch, next_wait};
pub use store::{RESETS_SCHEMA_VERSION, ResetsConfig, ResetsStore};
pub use watch::{Announcement, Markers, announce};
