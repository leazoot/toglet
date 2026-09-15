//! Notification channels. [`ChannelConfig`] is the displayable half kept in a plain file;
//! [`Connection`] holds the secrets and never leaves the credential store once saved.
//! Values are validated before they are stored, not when they are used.

use serde::{Deserialize, Serialize};

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

const PHASE: Phase = Phase::Notify;

/// Room for a signed webhook value; a Telegram bot token is about 46 characters.
const MAX_KEY_LEN: usize = 256;
const MAX_LABEL_CHARS: usize = 32;
const MAX_HOST_LEN: usize = 253;
/// The longest address RFC 5321 allows.
const MAX_ADDRESS_LEN: usize = 254;

/// Serialised by name so a reordered enum cannot turn one stored service into another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelKind {
    Bark,
    Wecom,
    Telegram,
    Webhook,
    Email,
}

impl ChannelKind {
    /// Stable wire form, shared with the interface. Append-only.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bark => "bark",
            Self::Wecom => "wecom",
            Self::Telegram => "telegram",
            Self::Webhook => "webhook",
            Self::Email => "email",
        }
    }

    /// The inverse. Unknown text is `None`, never the nearest match.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "bark" => Self::Bark,
            "wecom" => Self::Wecom,
            "telegram" => Self::Telegram,
            "webhook" => Self::Webhook,
            "email" => Self::Email,
            _ => return None,
        })
    }
}

/// Mail transport encryption. Deliberately no plaintext option: it would expose the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MailSecurity {
    /// TLS from the first byte - port 465.
    Tls,
    /// A plain connection upgraded with STARTTLS before authentication - port 587.
    StartTls,
}

impl MailSecurity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "startTls",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "tls" => Self::Tls,
            "startTls" => Self::StartTls,
            _ => return None,
        })
    }
}

/// Credential-store payload. For WeCom and plain webhooks the address itself is the credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Connection {
    #[serde(rename_all = "camelCase")]
    Bark { server: String, device_key: String },
    #[serde(rename_all = "camelCase")]
    Wecom { webhook: String },
    #[serde(rename_all = "camelCase")]
    Telegram {
        api_base: String,
        bot_token: String,
        chat_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Webhook { url: String },
    #[serde(rename_all = "camelCase")]
    Email {
        host: String,
        port: u16,
        security: MailSecurity,
        username: String,
        password: String,
        from: String,
        to: String,
    },
}

impl Connection {
    pub fn kind(&self) -> ChannelKind {
        match self {
            Self::Bark { .. } => ChannelKind::Bark,
            Self::Wecom { .. } => ChannelKind::Wecom,
            Self::Telegram { .. } => ChannelKind::Telegram,
            Self::Webhook { .. } => ChannelKind::Webhook,
            Self::Email { .. } => ChannelKind::Email,
        }
    }

    /// The displayable detail: the host only (the path may hold the token), or the masked
    /// recipient for e-mail.
    pub fn hint(&self) -> String {
        match self {
            Self::Bark { server, .. } => crate::net::host_of(server).unwrap_or_default().to_owned(),
            Self::Wecom { webhook } => crate::net::host_of(webhook).unwrap_or_default().to_owned(),
            Self::Telegram { api_base, .. } => {
                crate::net::host_of(api_base).unwrap_or_default().to_owned()
            }
            Self::Webhook { url } => crate::net::host_of(url).unwrap_or_default().to_owned(),
            Self::Email { to, .. } => crate::accounts::mask_email(to).unwrap_or_default(),
        }
    }

    /// Validates every field; one bad field rejects the whole connection.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Bark { server, device_key } => {
                check_url(server, "the Bark server address")?;
                check_key(device_key, "the Bark device key")
            }
            Self::Wecom { webhook } => check_url(webhook, "the WeCom webhook address"),
            Self::Telegram {
                api_base,
                bot_token,
                chat_id,
            } => {
                check_url(api_base, "the Telegram API address")?;
                check_key(bot_token, "the Telegram bot token")?;
                check_key(chat_id, "the Telegram chat id")
            }
            Self::Webhook { url } => check_url(url, "the webhook address"),
            Self::Email {
                host,
                port,
                username,
                password,
                from,
                to,
                security: _,
            } => {
                check_host(host)?;
                if *port == 0 {
                    return Err(rejected("the mail server port"));
                }
                check_key(username, "the mailbox user name")?;
                check_key(password, "the mailbox password")?;
                check_address(from, "the sender address")?;
                check_address(to, "the recipient address")
            }
        }
    }
}

/// Last delivery result. Holds only Toglet's stable code: a service's reply can quote the address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    /// Unix seconds.
    pub at: i64,
    pub ok: bool,
    pub code: Option<String>,
}

/// One channel, as the file on disk holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfig {
    pub id: String,
    pub kind: ChannelKind,
    /// User-chosen name, 1 to 32 characters; never translated, logged or passed to a command line.
    pub label: String,
    pub enabled: bool,
    /// See [`Connection::hint`].
    pub hint: String,
    pub created_at: String,
    pub last_delivery: Option<Delivery>,
}

/// Trims and checks a channel's name.
pub fn validate_label(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let length = trimmed.chars().count();
    if (1..=MAX_LABEL_CHARS).contains(&length) {
        Ok(trimmed.to_owned())
    } else {
        Err(rejected("the channel name"))
    }
}

/// `https` only, except `http` to loopback (a local Bark server or bridge has no certificate).
/// No whitespace or control characters that could split a request.
fn check_url(value: &str, what: &str) -> Result<()> {
    if crate::net::is_safe_endpoint(value) {
        Ok(())
    } else {
        Err(rejected(what))
    }
}

/// Printable ASCII without spaces: these values are copied from a service's own page.
fn check_key(value: &str, what: &str) -> Result<()> {
    let usable = !value.is_empty()
        && value.len() <= MAX_KEY_LEN
        && value.is_ascii()
        && !value.bytes().any(|byte| byte <= b' ' || byte == 0x7f);
    if usable { Ok(()) } else { Err(rejected(what)) }
}

fn check_host(value: &str) -> Result<()> {
    let usable = !value.is_empty()
        && value.len() <= MAX_HOST_LEN
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
    if usable {
        Ok(())
    } else {
        Err(rejected("the mail server address"))
    }
}

/// Deliberately loose (one `@`, a dotted domain, no whitespace); the mail server decides.
fn check_address(value: &str, what: &str) -> Result<()> {
    let Some((local, domain)) = value.split_once('@') else {
        return Err(rejected(what));
    };
    let usable = !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && value.len() <= MAX_ADDRESS_LEN
        && !value.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
        && value.matches('@').count() == 1;
    if usable { Ok(()) } else { Err(rejected(what)) }
}

/// Never quotes the value: errors reach the interface and the log.
fn rejected(what: &str) -> TogletError {
    TogletError::new(
        ErrorCode::Internal,
        PHASE,
        false,
        UserAction::FixNotificationChannel,
    )
    .with_detail(&format!("{what} is not in a usable form"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bark() -> Connection {
        Connection::Bark {
            server: "https://api.day.app".to_owned(),
            device_key: "AbCdEf123456".to_owned(),
        }
    }

    #[test]
    fn every_kind_name_parses_back_to_the_kind_that_produced_it() {
        for kind in [
            ChannelKind::Bark,
            ChannelKind::Wecom,
            ChannelKind::Telegram,
            ChannelKind::Webhook,
            ChannelKind::Email,
        ] {
            assert_eq!(ChannelKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ChannelKind::parse("slack"), None);
    }

    #[test]
    fn a_plain_http_address_is_refused_unless_it_is_this_machine() {
        assert!(check_url("http://hooks.example.com/abc", "webhook").is_err());
        assert!(check_url("http://127.0.0.1:8080/push", "webhook").is_ok());
        assert!(check_url("http://localhost/push", "webhook").is_ok());
    }

    #[test]
    fn an_address_that_could_split_a_request_is_refused() {
        assert!(check_url("https://example.com/a\r\nHost: evil", "webhook").is_err());
        assert!(check_url("https://example.com/a b", "webhook").is_err());
        assert!(check_url("ftp://example.com/a", "webhook").is_err());
        assert!(check_url("https://", "webhook").is_err());
        assert!(check_url("https:///path", "webhook").is_err());
    }

    #[test]
    fn a_hint_names_the_host_and_never_the_path_that_holds_the_token() {
        let wecom = Connection::Wecom {
            webhook: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=6f0e-secret".to_owned(),
        };
        assert_eq!(wecom.hint(), "qyapi.weixin.qq.com");
        assert!(!wecom.hint().contains("secret"));
    }

    #[test]
    fn a_hint_for_mail_is_the_masked_recipient() {
        let email = Connection::Email {
            host: "smtp.example.com".to_owned(),
            port: 465,
            security: MailSecurity::Tls,
            username: "leanne".to_owned(),
            password: "hunter2".to_owned(),
            from: "leanne@example.com".to_owned(),
            to: "leanne@example.com".to_owned(),
        };
        assert_eq!(email.hint(), "lea***@example.com");
    }

    #[test]
    fn a_valid_connection_passes_and_an_empty_key_does_not() {
        bark().validate().expect("a complete channel is usable");
        let empty = Connection::Bark {
            server: "https://api.day.app".to_owned(),
            device_key: String::new(),
        };
        assert!(empty.validate().is_err());
    }

    #[test]
    fn a_mail_channel_needs_both_addresses_and_a_port() {
        let base = Connection::Email {
            host: "smtp.example.com".to_owned(),
            port: 0,
            security: MailSecurity::StartTls,
            username: "leanne".to_owned(),
            password: "hunter2".to_owned(),
            from: "leanne@example.com".to_owned(),
            to: "team@example.com".to_owned(),
        };
        assert!(base.validate().is_err(), "port 0 reaches nothing");

        let Connection::Email { host, .. } = &base else {
            unreachable!("constructed as e-mail")
        };
        let good = Connection::Email {
            host: host.clone(),
            port: 587,
            security: MailSecurity::StartTls,
            username: "leanne".to_owned(),
            password: "hunter2".to_owned(),
            from: "leanne@example.com".to_owned(),
            to: "team@example.com".to_owned(),
        };
        good.validate().expect("a complete mailbox is usable");
    }

    #[test]
    fn an_address_without_a_domain_is_refused() {
        assert!(check_address("leanne@localhost", "recipient").is_err());
        assert!(check_address("leanne", "recipient").is_err());
        assert!(check_address("a@b@example.com", "recipient").is_err());
        check_address("leanne@example.com", "recipient").expect("ordinary address");
    }

    #[test]
    fn a_name_is_trimmed_and_bounded() {
        assert_eq!(validate_label("  Phone  ").expect("valid"), "Phone");
        assert!(validate_label("   ").is_err());
        assert!(validate_label(&"x".repeat(MAX_LABEL_CHARS + 1)).is_err());
    }

    #[test]
    fn an_error_about_a_value_never_quotes_the_value() {
        let error = check_key("", "the Telegram bot token").expect_err("refused");
        let detail = error.detail().unwrap_or_default();
        assert!(detail.contains("Telegram bot token"));
        assert!(!detail.contains("https"));
    }

    #[test]
    fn a_connection_round_trips_through_json_with_its_kind_named() {
        let json = serde_json::to_string(&bark()).expect("serialises");
        assert!(json.contains(r#""kind":"bark""#));
        let back: Connection = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, bark());
        assert_eq!(back.kind(), ChannelKind::Bark);
    }
}
