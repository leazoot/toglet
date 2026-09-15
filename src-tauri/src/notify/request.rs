//! Pure per-service request construction, so each envelope is testable without a network.
//! Every body is JSON over POST, so no user value is ever percent-encoded.

use serde_json::{Value, json};

use super::channel::{ChannelKind, Connection};

/// Notification text built by the interface; capped because it crosses the IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    title: String,
    body: String,
}

pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_BODY_CHARS: usize = 1000;

impl Message {
    /// Trims and truncates both parts; an over-long message is shortened, not refused.
    pub fn new(title: &str, body: &str) -> Self {
        Self {
            title: clip(title, MAX_TITLE_CHARS),
            body: clip(body, MAX_BODY_CHARS),
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    /// The two parts as one block, for the services that take a single text field.
    fn text(&self) -> String {
        if self.body.is_empty() {
            self.title.clone()
        } else {
            format!("{}\n{}", self.title, self.body)
        }
    }
}

fn clip(value: &str, cap: usize) -> String {
    value.trim().chars().take(cap).collect()
}

/// One outbound request: where it goes and what it carries. Always POST, always JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpCall {
    pub url: String,
    pub body: String,
}

/// Builds the HTTP request; `None` for e-mail, which `send::mail` handles.
pub fn build(connection: &Connection, message: &Message) -> Option<HttpCall> {
    let call = match connection {
        Connection::Bark { server, device_key } => HttpCall {
            url: format!("{}/push", server.trim_end_matches('/')),
            body: body(&json!({
                "device_key": device_key,
                "title": message.title(),
                "body": message.body(),
            })),
        },
        // A WeCom bot may require a keyword; the title is the product name, so "Toglet" works.
        Connection::Wecom { webhook } => HttpCall {
            url: webhook.clone(),
            body: body(&json!({
                "msgtype": "text",
                "text": { "content": message.text() },
            })),
        },
        Connection::Telegram {
            api_base,
            bot_token,
            chat_id,
        } => HttpCall {
            url: format!(
                "{}/bot{bot_token}/sendMessage",
                api_base.trim_end_matches('/')
            ),
            body: body(&json!({
                "chat_id": chat_id,
                "text": message.text(),
                // No `parse_mode`: a display name could hold characters a markup parser rejects.
                "disable_web_page_preview": true,
            })),
        },
        // Named fields rather than a sentence, so the user's own receiver can lay it out.
        Connection::Webhook { url } => HttpCall {
            url: url.clone(),
            body: body(&json!({
                "source": "toglet",
                "title": message.title(),
                "body": message.body(),
            })),
        },
        Connection::Email { .. } => return None,
    };
    Some(call)
}

/// Whether a 2xx reply means acceptance: WeCom, Bark and Telegram can answer `200 OK` with a
/// refusal in the body. An unparseable reply counts as accepted.
pub fn accepted(kind: ChannelKind, reply: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(reply) else {
        return true;
    };
    match kind {
        ChannelKind::Bark => number(&value, "code").is_none_or(|code| code == 200.0),
        ChannelKind::Wecom => number(&value, "errcode").is_none_or(|code| code == 0.0),
        ChannelKind::Telegram => value.get("ok").and_then(Value::as_bool).unwrap_or(true),
        // Nothing is promised about a bridge's reply; the status code is the answer.
        ChannelKind::Webhook | ChannelKind::Email => true,
    }
}

fn number(value: &Value, field: &str) -> Option<f64> {
    value.get(field).and_then(Value::as_f64)
}

/// Serialising a `json!` value of strings, numbers and bools cannot fail.
fn body(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| String::from("{}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> Message {
        Message::new("Toglet", "Switched to Work and carried on.")
    }

    fn call(connection: &Connection) -> HttpCall {
        build(connection, &message()).expect("an HTTP channel")
    }

    #[test]
    fn bark_posts_the_key_in_the_body_rather_than_in_the_address() {
        let built = call(&Connection::Bark {
            server: "https://api.day.app/".to_owned(),
            device_key: "AbCdEf".to_owned(),
        });

        assert_eq!(built.url, "https://api.day.app/push");
        assert!(built.body.contains(r#""device_key":"AbCdEf""#));
        assert!(built.body.contains(r#""title":"Toglet""#));
    }

    #[test]
    fn wecom_posts_to_the_address_it_was_given_in_its_own_envelope() {
        let wecom = call(&Connection::Wecom {
            webhook: "https://qyapi.weixin.qq.com/hook/x".to_owned(),
        });
        assert_eq!(wecom.url, "https://qyapi.weixin.qq.com/hook/x");
        assert!(wecom.body.contains(r#""msgtype":"text""#));
    }

    #[test]
    fn telegram_puts_the_token_in_the_path_and_the_chat_in_the_body() {
        let built = call(&Connection::Telegram {
            api_base: "https://api.telegram.org".to_owned(),
            bot_token: "123:AAE".to_owned(),
            chat_id: "-1001".to_owned(),
        });

        assert_eq!(built.url, "https://api.telegram.org/bot123:AAE/sendMessage");
        assert!(built.body.contains(r#""chat_id":"-1001""#));
    }

    #[test]
    fn the_single_text_services_carry_both_halves() {
        let built = call(&Connection::Wecom {
            webhook: "https://qyapi.weixin.qq.com/hook/x".to_owned(),
        });
        assert!(built.body.contains("Toglet"));
        assert!(built.body.contains("carried on"));
    }

    #[test]
    fn e_mail_has_no_http_request() {
        let email = Connection::Email {
            host: "smtp.example.com".to_owned(),
            port: 465,
            security: super::super::MailSecurity::Tls,
            username: "leanne".to_owned(),
            password: "hunter2".to_owned(),
            from: "leanne@example.com".to_owned(),
            to: "team@example.com".to_owned(),
        };
        assert!(build(&email, &message()).is_none());
    }

    #[test]
    fn a_long_message_is_cut_rather_than_refused() {
        let long = Message::new("t", &"x".repeat(MAX_BODY_CHARS + 500));
        assert_eq!(long.body().chars().count(), MAX_BODY_CHARS);
    }

    #[test]
    fn a_refusal_dressed_as_success_is_read_as_a_refusal() {
        assert!(!accepted(
            ChannelKind::Wecom,
            r#"{"errcode":93000,"errmsg":"no"}"#
        ));
        assert!(accepted(ChannelKind::Wecom, r#"{"errcode":0}"#));
        assert!(!accepted(ChannelKind::Telegram, r#"{"ok":false}"#));
        assert!(!accepted(ChannelKind::Bark, r#"{"code":400}"#));
        assert!(accepted(ChannelKind::Bark, r#"{"code":200}"#));
    }

    #[test]
    fn a_reply_that_says_nothing_recognisable_is_taken_at_its_status_code() {
        assert!(accepted(ChannelKind::Wecom, "ok"));
        assert!(accepted(ChannelKind::Telegram, ""));
        assert!(accepted(ChannelKind::Webhook, r#"{"code":19021}"#));
    }
}
