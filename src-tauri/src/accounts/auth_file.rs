//! Reading only the non-secret facts out of an `auth.json`.
//!
//! The file is full of tokens. The deserialisation target has **three optional fields and
//! nothing else**, so serde skips the rest on the stream: the access and refresh tokens are
//! never turned into owned strings, and therefore cannot end up in a buffer, an error or a log.
//!
//! The one token that is looked at is the id token, and only its payload: that is where the
//! account's own display name lives (the `name` claim, present on the ChatGPT sign-in
//! measured on 2026-09-02), and it is the only source of one - `account/read` returns an
//! address and a plan and nothing else. The token is borrowed from the buffer, its payload
//! is decoded into a [`Secret`] that is wiped when it goes out of scope, and one claim comes
//! out.

use std::borrow::Cow;

use serde::Deserialize;

use super::fingerprint;
use crate::credentials::Secret;

/// What can be learned from a credential file without keeping a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthFacts {
    /// `apikey` / `chatgpt` / `chatgptAuthTokens`, or whatever a future build writes.
    pub auth_mode: Option<String>,
    /// Irreversible derivation of the stable account id, when the file carries one.
    pub fingerprint: Option<String>,
    /// The name the account carries at ChatGPT, when the id token says one. Not validated as
    /// a display name here: that is the profile's job, and a name too long for a profile is
    /// still a fact about the account.
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
struct AuthShape<'a> {
    #[serde(default)]
    auth_mode: Option<String>,
    #[serde(default, borrow)]
    tokens: Option<TokensShape<'a>>,
}

#[derive(Deserialize)]
struct TokensShape<'a> {
    /// `access_token` and `refresh_token` are deliberately absent from this struct.
    #[serde(default)]
    account_id: Option<String>,
    /// Borrowed rather than owned: a JWT has no characters that need escaping, so serde hands
    /// back a slice of the buffer the caller already holds, and the `Cow` only ever owns if a
    /// file were to carry one that does.
    #[serde(default, borrow)]
    id_token: Option<Cow<'a, str>>,
}

/// The one claim read from the id token.
#[derive(Deserialize)]
struct NameClaim {
    #[serde(default)]
    name: Option<String>,
}

/// Extracts the auth mode, the fingerprint and the account's own name. Returns `None` if the
/// file is not JSON.
///
/// The fingerprint is derived here rather than the raw id being returned, so the caller never
/// holds the stable account id in the first place.
pub fn read(secret: &Secret) -> Option<AuthFacts> {
    let shape: AuthShape<'_> = serde_json::from_slice(secret.expose()).ok()?;

    let (account_id, id_token) = match shape.tokens {
        Some(tokens) => (tokens.account_id, tokens.id_token),
        None => (None, None),
    };
    Some(AuthFacts {
        auth_mode: shape.auth_mode,
        fingerprint: account_id
            .filter(|id| !id.trim().is_empty())
            .map(|id| fingerprint::from_account_id(&id)),
        display_name: id_token.and_then(|token| name_claim(&token)),
    })
}

/// The `name` claim of a JWT's payload, or `None` when there is none or the token is not one.
///
/// The signature is not checked. The claim is used for a label the user can change, not for
/// any decision about identity - identity is what a running app server says (`verify`).
fn name_claim(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    // The decoded payload also carries the address and the account ids, so it lives in a
    // buffer that is wiped on drop, like everything else read out of this file.
    let claims = Secret::new(base64url_decode(payload)?);
    let parsed: NameClaim = serde_json::from_slice(claims.expose()).ok()?;
    parsed
        .name
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// Decodes unpadded base64url (RFC 4648 §5), which is what JWT segments use.
///
/// Written here rather than taken from a crate: it is twenty lines, the input is one segment
/// of one token, and a dependency for it would be the larger review burden.
fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            // Padding is not part of the JWT alphabet, but a token that carries it is not
            // wrong, only unusual.
            b'=' => break,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            // The shift leaves exactly one byte in range, so the truncation is the point.
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_ID: &str = "8f14e45f-ceea-467a-9f3a-1c2d3e4f5a6b";

    /// Base64url of `{"sub":"user-1","email":"leanne@example.com","name":"Leanne Q"}`.
    const NAMED_PAYLOAD: &str =
        "eyJzdWIiOiJ1c2VyLTEiLCJlbWFpbCI6ImxlYW5uZUBleGFtcGxlLmNvbSIsIm5hbWUiOiJMZWFubmUgUSJ9";
    /// Base64url of `{"sub":"user-1","email":"leanne@example.com"}`.
    const UNNAMED_PAYLOAD: &str = "eyJzdWIiOiJ1c2VyLTEiLCJlbWFpbCI6ImxlYW5uZUBleGFtcGxlLmNvbSJ9";

    fn auth_json_with(id_token_payload: &str) -> Secret {
        Secret::new(
            format!(
                r#"{{"OPENAI_API_KEY":null,"auth_mode":"chatgpt","last_refresh":"2026-08-31T00:00:00Z",
                   "tokens":{{"access_token":"eyJhbGciOiJIUzI1NiJ9.aaaaaaaaaaaaaaaaaaaa",
                   "account_id":"{ACCOUNT_ID}","id_token":"eyJhbGciOiJIUzI1NiJ9.{id_token_payload}.sig",
                   "refresh_token":"rt-cccccccccccccccccccc"}}}}"#
            )
            .into_bytes(),
        )
    }

    fn auth_json() -> Secret {
        auth_json_with("bbbbbbbbbbbbbbbbbbbb")
    }

    #[test]
    fn the_account_name_is_read_from_the_id_token() {
        let facts = read(&auth_json_with(NAMED_PAYLOAD)).expect("the file parses");

        assert_eq!(facts.display_name.as_deref(), Some("Leanne Q"));
    }

    #[test]
    fn a_token_without_a_name_claim_yields_no_name_rather_than_an_invented_one() {
        let facts = read(&auth_json_with(UNNAMED_PAYLOAD)).expect("the file parses");

        assert_eq!(facts.display_name, None);
    }

    #[test]
    fn a_payload_that_is_not_a_token_is_not_a_name_either() {
        let facts = read(&auth_json()).expect("the file parses");

        assert_eq!(facts.display_name, None);
    }

    #[test]
    fn base64url_handles_every_remainder() {
        assert_eq!(base64url_decode("").expect("empty"), b"");
        assert_eq!(base64url_decode("Zg").expect("one byte"), b"f");
        assert_eq!(base64url_decode("Zm8").expect("two bytes"), b"fo");
        assert_eq!(base64url_decode("Zm9v").expect("three bytes"), b"foo");
        assert_eq!(base64url_decode("Zm9vYg==").expect("padded"), b"foob");
        assert_eq!(
            base64url_decode("-_8").expect("url alphabet"),
            &[0xfb, 0xff]
        );
        assert_eq!(base64url_decode("Zm9v!"), None);
    }

    #[test]
    fn the_auth_mode_and_fingerprint_are_read() {
        let facts = read(&auth_json()).expect("the file parses");

        assert_eq!(facts.auth_mode.as_deref(), Some("chatgpt"));
        assert_eq!(
            facts.fingerprint.as_deref(),
            Some(fingerprint::from_account_id(ACCOUNT_ID).as_str())
        );
    }

    #[test]
    fn the_fingerprint_does_not_contain_the_account_id() {
        let facts = read(&auth_json()).expect("the file parses");
        let fingerprint = facts.fingerprint.expect("present");

        assert!(!fingerprint.contains(ACCOUNT_ID));
        assert!(!fingerprint.contains("8f14e45f"));
    }

    #[test]
    fn nothing_read_out_resembles_a_token() {
        let facts = read(&auth_json()).expect("the file parses");

        let rendered = format!("{facts:?}");
        for secret in [
            "eyJ",
            "rt-cccc",
            "access_token",
            "refresh_token",
            "id_token",
        ] {
            assert!(!rendered.contains(secret), "the facts carried {secret}");
        }
    }

    #[test]
    fn an_api_key_file_reports_its_mode_and_no_fingerprint() {
        let secret = Secret::new(br#"{"OPENAI_API_KEY":"sk-x","auth_mode":"apikey"}"#.to_vec());

        let facts = read(&secret).expect("the file parses");

        assert_eq!(facts.auth_mode.as_deref(), Some("apikey"));
        assert_eq!(facts.fingerprint, None);
    }

    #[test]
    fn a_file_without_a_stable_id_yields_no_fingerprint_rather_than_an_invented_one() {
        let secret =
            Secret::new(br#"{"auth_mode":"chatgpt","tokens":{"account_id":"  "}}"#.to_vec());

        assert_eq!(read(&secret).expect("parses").fingerprint, None);
    }

    #[test]
    fn something_that_is_not_json_reads_as_nothing() {
        assert_eq!(read(&Secret::new(b"not json".to_vec())), None);
    }
}
