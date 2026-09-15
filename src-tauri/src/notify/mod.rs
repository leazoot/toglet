//! Task notifications: one title and body posted to channels the user configured.
//! Inert until a channel is added. Channel secrets stay in the credential store, never in
//! `notifications.json`, logs, errors or IPC replies; only `send` opens a connection.

pub mod channel;
mod dispatch;
mod request;
mod send;
mod store;

pub use channel::{ChannelConfig, ChannelKind, Connection, Delivery, MailSecurity, validate_label};
pub use dispatch::{Outcome, Target, deliver_all, prepare};
pub use request::{MAX_BODY_CHARS, MAX_TITLE_CHARS, Message};
pub use store::{
    CHANNELS_SCHEMA_VERSION, ChannelBook, ChannelStore, MAX_CHANNELS, forget_connection,
    load_connection, store_connection,
};
