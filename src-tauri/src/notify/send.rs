//! Opens the notification connection: posts a title and body to an address the user entered.
//! Nothing but the message travels; there is no Toglet server at the other end.

use std::sync::OnceLock;
use std::time::Duration;

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use super::channel::{Connection, MailSecurity};
use super::request::{self, Message};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

const PHASE: Phase = Phase::Notify;

/// Single attempt, no retry: a late notification is stale, and retries can get an address blocked.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Sends one message through one channel.
pub async fn deliver(connection: &Connection, message: &Message) -> Result<()> {
    match request::build(connection, message) {
        Some(call) => post(connection, &call).await,
        None => mail(connection, message).await,
    }
}

async fn post(connection: &Connection, call: &request::HttpCall) -> Result<()> {
    let response = client()?
        .post(&call.url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(call.body.clone())
        .send()
        .await
        .map_err(|error| unreachable_service(&error.to_string()))?;

    let status = response.status();
    // Read the body even on success: some services answer `200 OK` and refuse in the body.
    let reply = response.text().await.unwrap_or_default();

    if status.is_server_error() {
        return Err(unreachable_service(
            "the service reported a failure of its own",
        ));
    }
    if !status.is_success() {
        return Err(refused("the service did not accept the request"));
    }
    if request::accepted(connection.kind(), &reply) {
        Ok(())
    } else {
        Err(refused("the service answered that it refused the message"))
    }
}

async fn mail(connection: &Connection, message: &Message) -> Result<()> {
    let Connection::Email {
        host,
        port,
        security,
        username,
        password,
        from,
        to,
    } = connection
    else {
        // Unreachable: `request::build` returns `None` only for e-mail. An error, not a panic.
        return Err(refused("this channel is not an e-mail channel"));
    };

    let envelope = lettre::Message::builder()
        .from(mailbox(from)?)
        .to(mailbox(to)?)
        .subject(message.title())
        .body(message.body().to_owned())
        .map_err(|error| refused(&error.to_string()))?;

    // Both paths are encrypted: `relay` is TLS from the first byte (465), `starttls_relay`
    // upgrades before authenticating (587). There is no unencrypted path.
    let builder = match security {
        MailSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host),
        MailSecurity::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host),
    }
    .map_err(|error| unreachable_service(&error.to_string()))?;

    let transport = builder
        .port(*port)
        .credentials(Credentials::new(username.clone(), password.clone()))
        .timeout(Some(TIMEOUT))
        .build();

    transport
        .send(envelope)
        .await
        .map_err(|error| mail_failure(&error))?;
    Ok(())
}

fn mailbox(address: &str) -> Result<Mailbox> {
    address
        .parse::<Mailbox>()
        .map_err(|_| refused("an address the mail server would not accept"))
}

/// Permanent and client-side SMTP errors mean "fix the channel", not "check the network".
fn mail_failure(error: &lettre::transport::smtp::Error) -> TogletError {
    if error.is_permanent() || error.is_client() {
        refused(&error.to_string())
    } else {
        unreachable_service(&error.to_string())
    }
}

/// Shared client. Redirects are refused so a body carrying a device key is never re-sent elsewhere.
fn client() -> Result<&'static reqwest::Client> {
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

/// Unreachable service or server-side failure: retryable, the channel itself is fine.
fn unreachable_service(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::NetworkUnavailable,
        PHASE,
        true,
        UserAction::CheckNetwork,
    )
    .with_detail(detail)
}

/// The service answered and refused: not retryable, the channel needs fixing.
fn refused(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::NotificationRejected,
        PHASE,
        false,
        UserAction::FixNotificationChannel,
    )
    .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_that_says_no_is_not_reported_as_a_network_problem() {
        let error = refused("the service answered that it refused the message");
        assert_eq!(error.code(), ErrorCode::NotificationRejected);
        assert!(!error.retryable());
        assert_eq!(error.action(), UserAction::FixNotificationChannel);
    }

    #[test]
    fn an_unreachable_service_is_worth_another_try() {
        let error = unreachable_service("timed out");
        assert_eq!(error.code(), ErrorCode::NetworkUnavailable);
        assert!(error.retryable());
        assert_eq!(error.action(), UserAction::CheckNetwork);
    }

    /// Transport errors quote the failing address; the detail must pass through the redactor.
    #[test]
    fn an_address_inside_a_transport_failure_never_survives_into_the_error() {
        let error = unreachable_service(
            "error sending request for url (https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=aa-bb)",
        );
        let detail = error.detail().unwrap_or_default();
        assert!(!detail.contains("weixin"));
        assert!(!detail.contains("aa-bb"));

        let mail = refused("550 5.1.1 <leanne@example.com>: recipient rejected");
        let detail = mail.detail().unwrap_or_default();
        assert!(!detail.contains("leanne@example.com"));
    }

    #[test]
    fn an_address_the_mail_server_would_refuse_is_caught_before_a_connection_is_opened() {
        assert!(mailbox("not an address").is_err());
        mailbox("leanne@example.com").expect("an ordinary address parses");
    }
}
