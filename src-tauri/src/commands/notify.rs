//! Notification channel commands. Connection details are sent once when typed and never
//! returned: the interface only sees a host hint. There is no command that reads them back,
//! because there is no plaintext export.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::State;

use super::state::AppState;
use super::views::ErrorView;
use crate::autorun::rfc3339;
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::notify::{
    ChannelBook, ChannelConfig, ChannelStore, Connection, Delivery, MAX_CHANNELS, MailSecurity,
    Message, deliver_all, forget_connection, prepare, store_connection, validate_label,
};
use crate::storage::LoadOutcome;

const PHASE: Phase = Phase::Notify;

const BARK_DEFAULT_SERVER: &str = "https://api.day.app";
/// Overridable because a reverse proxy is the only way in where Telegram is blocked.
const TELEGRAM_DEFAULT_API: &str = "https://api.telegram.org";

/// One channel as shown; never anything that could post somewhere.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyChannelView {
    pub id: String,
    pub kind: &'static str,
    pub label: String,
    pub enabled: bool,
    /// The host, or a masked recipient for e-mail.
    pub hint: String,
    pub last_delivery: Option<DeliveryView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryView {
    /// Unix seconds.
    pub at: i64,
    pub ok: bool,
    /// Stable error code, never free text.
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyView {
    pub channels: Vec<NotifyChannelView>,
    pub max_channels: usize,
}

impl NotifyView {
    fn of(book: &ChannelBook) -> Self {
        Self {
            channels: book
                .channels
                .iter()
                .map(|channel| NotifyChannelView {
                    id: channel.id.clone(),
                    kind: channel.kind.as_str(),
                    label: channel.label.clone(),
                    enabled: channel.enabled,
                    hint: channel.hint.clone(),
                    last_delivery: channel.last_delivery.as_ref().map(|delivery| DeliveryView {
                        at: delivery.at,
                        ok: delivery.ok,
                        code: delivery.code.clone(),
                    }),
                })
                .collect(),
            max_channels: MAX_CHANNELS,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeView {
    pub channel_id: String,
    pub ok: bool,
    pub code: Option<String>,
}

/// Connection details as typed. Server defaults are filled in here only, never in the interface.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ConnectionInput {
    #[serde(rename_all = "camelCase")]
    Bark {
        server: Option<String>,
        device_key: String,
    },
    #[serde(rename_all = "camelCase")]
    Wecom { webhook: String },
    #[serde(rename_all = "camelCase")]
    Telegram {
        api_base: Option<String>,
        bot_token: String,
        chat_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Webhook { url: String },
    #[serde(rename_all = "camelCase")]
    Email {
        host: String,
        port: u16,
        security: String,
        username: String,
        password: String,
        from: String,
        to: String,
    },
}

impl ConnectionInput {
    fn into_connection(self) -> Result<Connection> {
        Ok(match self {
            Self::Bark { server, device_key } => Connection::Bark {
                server: trimmed_or(server, BARK_DEFAULT_SERVER),
                device_key: device_key.trim().to_owned(),
            },
            Self::Wecom { webhook } => Connection::Wecom {
                webhook: webhook.trim().to_owned(),
            },
            Self::Telegram {
                api_base,
                bot_token,
                chat_id,
            } => Connection::Telegram {
                api_base: trimmed_or(api_base, TELEGRAM_DEFAULT_API),
                bot_token: bot_token.trim().to_owned(),
                chat_id: chat_id.trim().to_owned(),
            },
            Self::Webhook { url } => Connection::Webhook {
                url: url.trim().to_owned(),
            },
            Self::Email {
                host,
                port,
                security,
                username,
                password,
                from,
                to,
            } => Connection::Email {
                host: host.trim().to_owned(),
                port,
                security: MailSecurity::parse(&security)
                    .ok_or_else(|| rejected("the connection security"))?,
                username: username.trim().to_owned(),
                // Not trimmed: a password's spaces are part of it.
                password,
                from: from.trim().to_owned(),
                to: to.trim().to_owned(),
            },
        })
    }
}

fn trimmed_or(value: Option<String>, fallback: &str) -> String {
    match value {
        Some(value) if !value.trim().is_empty() => value.trim().trim_end_matches('/').to_owned(),
        _ => fallback.to_owned(),
    }
}

/// `id` absent adds a channel; `connection` absent keeps the stored details.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveChannelRequest {
    pub id: Option<String>,
    pub label: String,
    pub enabled: bool,
    pub connection: Option<ConnectionInput>,
}

pub struct Notifications {
    store: ChannelStore,
    book: Mutex<ChannelBook>,
}

impl Notifications {
    /// A damaged channel file is logged and replaced with an empty list; start-up continues.
    pub fn load(data_directory: &std::path::Path) -> Self {
        let store = ChannelStore::new(data_directory);
        let (book, outcome) = store.load();
        if let LoadOutcome::Rebuilt { .. } = outcome {
            log(&LogRecord::new(Level::Warn, "notify_channels_rebuilt_at_start").with_phase(PHASE));
        }
        Self {
            store,
            book: Mutex::new(book),
        }
    }

    /// The ids of every channel that exists, whatever its switch says. For features that pick
    /// channels by id (reset alerts): an id whose channel is gone must be dropped, not sent to.
    pub fn channel_ids(&self) -> Vec<String> {
        self.book()
            .channels
            .iter()
            .map(|channel| channel.id.clone())
            .collect()
    }

    fn book(&self) -> std::sync::MutexGuard<'_, ChannelBook> {
        self.book
            .lock()
            // A plain list cannot be left inconsistent by a panic, so poisoning is ignored.
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `action` against the list and saves it if the action changed anything.
    fn with_book<T>(
        &self,
        action: impl FnOnce(&mut ChannelBook) -> Result<(T, bool)>,
    ) -> Result<T> {
        let mut book = self.book();
        let (value, changed) = action(&mut book)?;
        if changed {
            self.store.save(&book)?;
        }
        Ok(value)
    }
}

#[tauri::command]
pub fn read_notify_channels(notifications: State<'_, Notifications>) -> NotifyView {
    NotifyView::of(&notifications.book())
}

#[tauri::command]
pub fn save_notify_channel(
    notifications: State<'_, Notifications>,
    state: State<'_, AppState>,
    request: SaveChannelRequest,
) -> std::result::Result<NotifyView, ErrorView> {
    reported(
        "notify_channel_not_saved",
        save(&notifications, &state, request),
    )
}

fn save(
    notifications: &Notifications,
    state: &AppState,
    request: SaveChannelRequest,
) -> Result<NotifyView> {
    let label = validate_label(&request.label)?;
    let connection = request
        .connection
        .map(ConnectionInput::into_connection)
        .transpose()?;
    let now = unix_seconds();

    // Details are stored before the list changes, so no entry exists whose details failed to store.
    notifications.with_book(|book| {
        match request.id {
            Some(id) => {
                let existing = book
                    .find(&id)
                    .ok_or_else(|| rejected("the channel"))?
                    .clone();
                let (kind, hint) = match &connection {
                    Some(connection) => {
                        store_connection(state.secrets(), &id, connection)?;
                        (connection.kind(), connection.hint())
                    }
                    None => (existing.kind, existing.hint.clone()),
                };
                let channel = book.find_mut(&id).ok_or_else(|| rejected("the channel"))?;
                channel.label = label;
                channel.enabled = request.enabled;
                channel.kind = kind;
                channel.hint = hint;
            }
            None => {
                let connection = connection.ok_or_else(|| rejected("the channel details"))?;
                if book.channels.len() >= MAX_CHANNELS {
                    return Err(TogletError::new(
                        ErrorCode::Internal,
                        PHASE,
                        false,
                        UserAction::None,
                    )
                    .with_detail("no more channels can be added"));
                }
                let id = book.fresh_id(now);
                store_connection(state.secrets(), &id, &connection)?;
                book.channels.push(ChannelConfig {
                    id,
                    kind: connection.kind(),
                    label,
                    enabled: request.enabled,
                    hint: connection.hint(),
                    created_at: rfc3339(now),
                    last_delivery: None,
                });
            }
        }
        Ok((NotifyView::of(book), true))
    })
}

#[tauri::command]
pub fn remove_notify_channel(
    notifications: State<'_, Notifications>,
    state: State<'_, AppState>,
    channel_id: String,
) -> std::result::Result<NotifyView, ErrorView> {
    reported(
        "notify_channel_not_removed",
        notifications.with_book(|book| {
            let before = book.channels.len();
            book.channels.retain(|channel| channel.id != channel_id);
            if book.channels.len() == before {
                return Err(rejected("the channel"));
            }
            // After the list: a removed credential with a surviving entry would look configured.
            forget_connection(state.secrets(), &channel_id)?;
            Ok((NotifyView::of(book), true))
        }),
    )
}

/// Sends to every enabled channel, or only `channelId`. Title and body are capped here, not
/// trusted. An empty answer means nothing was configured.
// `async`: each channel gets up to ten seconds, and the main thread is the event loop.
#[tauri::command(async)]
pub async fn send_notification(
    notifications: State<'_, Notifications>,
    state: State<'_, AppState>,
    title: String,
    body: String,
    channel_id: Option<String>,
) -> std::result::Result<Vec<OutcomeView>, ErrorView> {
    let message = Message::new(&title, &body);
    // A copy, so the lock is not held across an await.
    let book = notifications.book().clone();
    if let Some(id) = channel_id.as_deref()
        && book.find(id).is_none()
    {
        return Err(ErrorView::from(rejected("the channel")));
    }

    // Credentials are read up front so the store is not held across the network wait.
    let targets = prepare(state.secrets(), &book, channel_id.as_deref());
    let now = unix_seconds();
    let results = deliver_all(targets, &message, now).await;

    record(&notifications, &results);

    Ok(results
        .into_iter()
        .map(|(outcome, _)| OutcomeView {
            channel_id: outcome.channel_id,
            ok: outcome.ok,
            code: outcome.code,
        })
        .collect())
}

/// Records each attempt. Best effort: a failed write does not turn a sent message into a failure.
fn record(notifications: &Notifications, results: &[(crate::notify::Outcome, Delivery)]) {
    if results.is_empty() {
        return;
    }
    let written = notifications.with_book(|book| {
        for (outcome, delivery) in results {
            if let Some(channel) = book.find_mut(&outcome.channel_id) {
                channel.last_delivery = Some(delivery.clone());
            }
        }
        Ok(((), true))
    });
    if let Err(error) = written {
        log(&LogRecord::from_error("notify_result_not_recorded", &error));
    }
}

/// Logs the redacted failure before it reaches the interface, which only shows the code.
fn reported<T>(event: &'static str, result: Result<T>) -> std::result::Result<T, ErrorView> {
    result.map_err(|error| {
        log(&LogRecord::from_error(event, &error));
        ErrorView::from(error)
    })
}

fn rejected(what: &str) -> TogletError {
    TogletError::new(
        ErrorCode::Internal,
        PHASE,
        false,
        UserAction::FixNotificationChannel,
    )
    .with_detail(&format!("{what} was not recognised"))
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::ChannelKind;

    fn input(json: &str) -> ConnectionInput {
        serde_json::from_str(json).expect("the form's shape parses")
    }

    #[test]
    fn an_empty_bark_server_falls_back_to_the_service_rather_than_to_nothing() {
        let connection = input(r#"{"kind":"bark","server":"  ","deviceKey":"AbCd"}"#)
            .into_connection()
            .expect("valid");

        assert_eq!(
            connection,
            Connection::Bark {
                server: BARK_DEFAULT_SERVER.to_owned(),
                device_key: "AbCd".to_owned(),
            }
        );
        connection
            .validate()
            .expect("the default service is usable");
    }

    #[test]
    fn a_trailing_slash_on_a_server_is_dropped_once_rather_than_twice_in_the_address() {
        let connection = input(r#"{"kind":"telegram","apiBase":"https://tg.example.com/","botToken":"1:A","chatId":"7"}"#)
            .into_connection()
            .expect("valid");

        assert_eq!(
            connection,
            Connection::Telegram {
                api_base: "https://tg.example.com".to_owned(),
                bot_token: "1:A".to_owned(),
                chat_id: "7".to_owned(),
            }
        );
    }

    #[test]
    fn a_connection_security_the_product_does_not_define_is_refused() {
        let plain = input(
            r#"{"kind":"email","host":"smtp.example.com","port":25,"security":"none",
                "username":"leanne","password":"hunter2","from":"a@example.com","to":"b@example.com"}"#,
        );
        assert!(
            plain.into_connection().is_err(),
            "there is no plaintext SMTP"
        );
    }

    #[test]
    fn a_mailbox_password_keeps_the_spaces_that_are_part_of_it() {
        // App passwords come in space-separated groups of four.
        let connection = input(
            r#"{"kind":"email","host":"smtp.example.com","port":465,"security":"tls",
                "username":" leanne ","password":"abcd efgh ijkl mnop",
                "from":" a@example.com ","to":"b@example.com"}"#,
        )
        .into_connection()
        .expect("valid");

        let Connection::Email {
            username,
            password,
            from,
            ..
        } = &connection
        else {
            unreachable!("built as e-mail")
        };
        assert_eq!(username, "leanne");
        assert_eq!(from, "a@example.com");
        assert_eq!(password, "abcd efgh ijkl mnop");
    }

    #[test]
    fn a_field_the_form_does_not_define_is_refused_rather_than_ignored() {
        let extra = serde_json::from_str::<ConnectionInput>(
            r#"{"kind":"webhook","url":"https://example.com/h","headers":{"X":"y"}}"#,
        );
        assert!(extra.is_err());
    }

    #[test]
    fn the_view_of_a_channel_carries_nothing_that_could_post_anywhere() {
        let book = ChannelBook {
            schema_version: crate::notify::CHANNELS_SCHEMA_VERSION,
            channels: vec![ChannelConfig {
                id: "chan-1".to_owned(),
                kind: ChannelKind::Wecom,
                label: "Team".to_owned(),
                enabled: true,
                hint: "qyapi.weixin.qq.com".to_owned(),
                created_at: "2026-09-12T10:00:00Z".to_owned(),
                last_delivery: Some(Delivery {
                    at: 1_757_000_000,
                    ok: false,
                    code: Some("notification_rejected".to_owned()),
                }),
            }],
        };

        let json = serde_json::to_string(&NotifyView::of(&book)).expect("serialises");

        for forbidden in ["https://", "cgi-bin", "hook", "@", "password", "token"] {
            assert!(!json.contains(forbidden), "the view leaked {forbidden}");
        }
        assert!(json.contains("qyapi.weixin.qq.com"));
        assert!(json.contains("notification_rejected"));
    }
}
