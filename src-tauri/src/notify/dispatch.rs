//! Sends one message to every enabled channel. [`prepare`] reads credentials before any network
//! I/O; [`deliver_all`] sends sequentially, and a failing channel does not stop the rest.

use super::channel::{ChannelKind, Connection, Delivery};
use super::request::Message;
use super::store::ChannelBook;
use super::{send, store};
use crate::credentials::SecretStore;
use crate::diagnostics::{Level, LogRecord, Phase, Result, log};

const PHASE: Phase = Phase::Notify;

/// One channel with its connection already read; the plaintext lives only as long as the send.
pub struct Target {
    channel_id: String,
    kind: ChannelKind,
    /// A read failure is carried so the channel still reports an outcome instead of being skipped.
    connection: Result<Connection>,
}

/// What one channel did with one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub channel_id: String,
    pub ok: bool,
    /// Toglet's stable code, never the service's own words.
    pub code: Option<String>,
}

/// Picks every enabled channel, or with `only` the named one regardless of `enabled` (test sends).
pub fn prepare(secrets: &dyn SecretStore, book: &ChannelBook, only: Option<&str>) -> Vec<Target> {
    book.channels
        .iter()
        .filter(|channel| match only {
            Some(id) => channel.id == id,
            None => channel.enabled,
        })
        .map(|channel| Target {
            channel_id: channel.id.clone(),
            kind: channel.kind,
            connection: store::load_connection(secrets, &channel.id),
        })
        .collect()
}

/// Sends to each target in turn; one outcome per target, in order. An empty list is not a failure.
pub async fn deliver_all(
    targets: Vec<Target>,
    message: &Message,
    now: i64,
) -> Vec<(Outcome, Delivery)> {
    let mut outcomes = Vec::with_capacity(targets.len());
    for target in targets {
        let result = match &target.connection {
            Ok(connection) => send::deliver(connection, message).await,
            Err(error) => Err(error.clone()),
        };

        let code = match &result {
            Ok(()) => None,
            Err(error) => {
                // Log id, kind and code only: never the label, host or the service's reply.
                log(&LogRecord::new(Level::Warn, "notification_not_delivered")
                    .with_phase(PHASE)
                    .with_code(error.code())
                    .with_detail(&format!(
                        "{} via {}",
                        target.channel_id,
                        target.kind.as_str()
                    )));
                Some(error.code().as_str().to_owned())
            }
        };

        let ok = result.is_ok();
        outcomes.push((
            Outcome {
                channel_id: target.channel_id,
                ok,
                code: code.clone(),
            },
            Delivery { at: now, ok, code },
        ));
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::MemorySecretStore;
    use crate::notify::channel::ChannelConfig;
    use crate::notify::store::CHANNELS_SCHEMA_VERSION;

    fn channel(id: &str, enabled: bool) -> ChannelConfig {
        ChannelConfig {
            id: id.to_owned(),
            kind: ChannelKind::Webhook,
            label: "Bridge".to_owned(),
            enabled,
            hint: "example.test".to_owned(),
            created_at: "2026-09-12T10:00:00Z".to_owned(),
            last_delivery: None,
        }
    }

    fn book(channels: Vec<ChannelConfig>) -> ChannelBook {
        ChannelBook {
            schema_version: CHANNELS_SCHEMA_VERSION,
            channels,
        }
    }

    /// Uses Tauri's runtime to avoid a test-only async dependency. No channel has a stored
    /// connection, so every attempt fails before reaching the network.
    fn run(
        secrets: &MemorySecretStore,
        book: &ChannelBook,
        only: Option<&str>,
        now: i64,
    ) -> Vec<(Outcome, Delivery)> {
        let targets = prepare(secrets, book, only);
        tauri::async_runtime::block_on(deliver_all(targets, &Message::new("t", "b"), now))
    }

    #[test]
    fn a_channel_that_is_switched_off_is_not_sent_to() {
        let secrets = MemorySecretStore::default();
        let book = book(vec![channel("chan-on", true), channel("chan-off", false)]);

        let outcomes = run(&secrets, &book, None, 1);

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].0.channel_id, "chan-on");
    }

    #[test]
    fn naming_a_channel_sends_to_it_even_when_it_is_switched_off() {
        let secrets = MemorySecretStore::default();
        let book = book(vec![channel("chan-on", true), channel("chan-off", false)]);

        let outcomes = run(&secrets, &book, Some("chan-off"), 1);

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].0.channel_id, "chan-off");
    }

    #[test]
    fn one_channel_failing_does_not_stop_the_next() {
        let secrets = MemorySecretStore::default();
        let book = book(vec![channel("chan-a", true), channel("chan-b", true)]);

        let outcomes = run(&secrets, &book, None, 7);

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(outcome, _)| !outcome.ok));
        assert!(outcomes.iter().all(|(_, delivery)| delivery.at == 7));
        assert!(outcomes.iter().all(|(outcome, _)| outcome.code.is_some()));
    }

    #[test]
    fn an_empty_list_sends_nothing_and_is_not_a_failure() {
        let secrets = MemorySecretStore::default();

        let outcomes = run(&secrets, &book(Vec::new()), None, 1);

        assert!(outcomes.is_empty());
    }

    #[test]
    fn preparing_reads_every_picked_channel_before_anything_is_sent() {
        let secrets = MemorySecretStore::default();
        let book = book(vec![channel("chan-a", true), channel("chan-b", true)]);

        let targets = prepare(&secrets, &book, None);

        assert_eq!(targets.len(), 2);
        assert!(
            targets.iter().all(|target| target.connection.is_err()),
            "nothing was stored for either, and that is carried rather than thrown"
        );
    }
}
