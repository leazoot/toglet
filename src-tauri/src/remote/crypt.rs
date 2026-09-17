//! Sealing the agent excerpt that rides in a remote receipt.
//!
//! HMAC proves who sent an envelope but hides nothing inside it, and the bridge is untrusted:
//! its status endpoint hands the last receipt to whoever asks. So the one piece of session
//! content allowed out - the excerpt - travels encrypted end to end.
//!
//! The key is derived from the same shared secret but under its own domain separator, so the
//! bytes that sign a receipt are never the bytes that seal one.

use aes_gcm::aead::{Aead, Generate, Key, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use sha2::{Digest, Sha256};

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

const PHASE: Phase = Phase::Remote;

/// Domain separator. Signing uses the raw secret; sealing must not, or one key would serve two
/// purposes and a weakness in either would cost both.
const EXCERPT_DOMAIN: &str = "toglet-remote/2 excerpt";

/// Domain separator for the bridge's read key. A third purpose, so a third key: see
/// [`status_key_hex`].
const STATUS_DOMAIN: &str = "toglet-remote/2 status";

/// AES-256-GCM takes a 96-bit nonce.
const NONCE_BYTES: usize = 12;

/// A sealed excerpt in the form it travels: both halves lower-case hex, both inside the
/// receipt's signed byte string.
#[derive(Clone, PartialEq, Eq)]
pub struct Sealed {
    pub ciphertext_hex: String,
    pub nonce_hex: String,
}

/// Says nothing. What this holds is session content once opened, and a captured value must not
/// be able to quote itself into a log line or an error detail.
impl std::fmt::Debug for Sealed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sealed(..)")
    }
}

/// Seals `plaintext` with a key derived from `secret`, using a nonce generated for this call.
///
/// A fresh nonce every time is not a nicety: GCM loses both confidentiality and integrity if a
/// nonce repeats under one key.
pub fn seal(secret: &[u8], plaintext: &str) -> Result<Sealed> {
    let key = Key::<Aes256Gcm>::from(sealing_key(secret));
    let cipher = Aes256Gcm::new(&key);

    let nonce_bytes = <[u8; NONCE_BYTES]>::try_generate()
        .map_err(|_| internal("the system random number generator was unavailable"))?;
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| internal("the excerpt could not be sealed"))?;

    Ok(Sealed {
        ciphertext_hex: to_hex(&ciphertext),
        nonce_hex: to_hex(&nonce_bytes),
    })
}

/// Opens what [`seal`] produced, or `None` when it does not authenticate.
///
/// `None` rather than an error, for the same reason `mac::verify_hex` returns a bool: a value
/// that fails to authenticate is expected behaviour on an untrusted path, not an internal fault.
/// GCM verifies the tag before returning anything, so a tampered ciphertext yields nothing at
/// all rather than plausible-looking rubbish.
pub fn open(secret: &[u8], sealed: &Sealed) -> Option<String> {
    let nonce_bytes: [u8; NONCE_BYTES] = from_hex(&sealed.nonce_hex)?.try_into().ok()?;
    let ciphertext = from_hex(&sealed.ciphertext_hex)?;

    let key = Key::<Aes256Gcm>::from(sealing_key(secret));
    let cipher = Aes256Gcm::new(&key);
    let plaintext = cipher
        .decrypt(&Nonce::from(nonce_bytes), ciphertext.as_slice())
        .ok()?;

    String::from_utf8(plaintext).ok()
}

/// The key a bridge checks a `GET /status` read against, as lower-case hex.
///
/// The bridge has to tell the user's phone from a stranger - the receipt it hands out can carry a
/// sealed excerpt - without becoming able to forge a command. A one-way derivation gives it
/// exactly that much: SHA-256 does not run backwards, the command MAC uses the raw secret this
/// value cannot reveal, and the excerpt uses [`EXCERPT_DOMAIN`] again. Three uses, three keys.
///
/// Unlike the secret, this is meant to be shown and pasted onto a machine the design treats as
/// untrusted, which is why it may cross to the interface. This is the only derivation in Rust;
/// `examples/remote-bridge/e2e.mjs` and the phone page compute the same value.
pub fn status_key_hex(secret: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(STATUS_DOMAIN.as_bytes());
    hasher.update(secret);
    let digest = hasher.finalize();
    to_hex(&digest)
}

/// The sealing key: SHA-256 over the domain separator and the shared secret.
fn sealing_key(secret: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EXCERPT_DOMAIN.as_bytes());
    hasher.update(secret);
    hasher.finalize().into()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decodes lower- or upper-case hex of any even length; `None` for anything else.
fn from_hex(hex: &str) -> Option<Vec<u8>> {
    // `% 2` rather than `is_multiple_of`, which needs a newer toolchain than the declared MSRV.
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut out = Vec::with_capacity(hex.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push((digit(pair[0])? << 4) | digit(pair[1])?);
    }
    Some(out)
}

fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn internal(detail: &str) -> TogletError {
    TogletError::new(ErrorCode::Internal, PHASE, true, UserAction::Retry).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"a-shared-secret-for-the-tests";

    #[test]
    fn what_was_sealed_comes_back_out() {
        for plaintext in [
            "",
            "x",
            "I will refactor the parser and run the tests.",
            "我会先把解析器重构掉，再跑一遍测试。",
            &"a".repeat(300),
        ] {
            let sealed = seal(SECRET, plaintext).expect("sealing works");
            assert_eq!(open(SECRET, &sealed).as_deref(), Some(plaintext));
        }
    }

    /// A repeated nonce under one key breaks GCM outright, so each call must draw a new one.
    #[test]
    fn the_same_text_seals_differently_every_time_and_both_open() {
        let first = seal(SECRET, "same text").expect("sealing works");
        let second = seal(SECRET, "same text").expect("sealing works");

        assert_ne!(first.nonce_hex, second.nonce_hex);
        assert_ne!(first.ciphertext_hex, second.ciphertext_hex);
        assert_eq!(open(SECRET, &first).as_deref(), Some("same text"));
        assert_eq!(open(SECRET, &second).as_deref(), Some("same text"));
    }

    #[test]
    fn a_tampered_ciphertext_yields_nothing_rather_than_rubbish() {
        let sealed = seal(SECRET, "the plan looks right to me").expect("sealing works");

        let mut tampered = sealed.clone();
        tampered.ciphertext_hex = flip_first_nibble(&sealed.ciphertext_hex);
        assert_eq!(open(SECRET, &tampered), None);
    }

    #[test]
    fn a_tampered_nonce_yields_nothing() {
        let sealed = seal(SECRET, "the plan looks right to me").expect("sealing works");

        let mut tampered = sealed.clone();
        tampered.nonce_hex = flip_first_nibble(&sealed.nonce_hex);
        assert_eq!(open(SECRET, &tampered), None);
    }

    #[test]
    fn another_secret_does_not_open_it() {
        let sealed = seal(SECRET, "the plan looks right to me").expect("sealing works");
        assert_eq!(open(b"a-different-secret", &sealed), None);
    }

    /// The bytes that sign a receipt must never be the bytes that seal one.
    #[test]
    fn the_sealing_key_is_not_the_shared_secret() {
        let derived = sealing_key(SECRET);
        assert_ne!(derived.as_slice(), SECRET);

        // And it is bound to its domain: the same secret under another separator differs.
        let mut other = Sha256::new();
        other.update(b"toglet-remote/2 something else");
        other.update(SECRET);
        let other: [u8; 32] = other.finalize().into();
        assert_ne!(derived, other);
    }

    /// A vector computed independently with `shasum -a 256` and with Node's `createHash`, so the
    /// desktop, the reference bridge and the phone page cannot drift apart on this derivation
    /// without a test saying so.
    #[test]
    fn the_status_key_matches_what_the_other_implementations_derive() {
        assert_eq!(
            status_key_hex(b"TEST-SECRET"),
            "184b2be9f1ed19cd39a53a95715a7ee4dca55a817e48aa4c20ac6d99279845ea"
        );
    }

    /// Three uses, three keys. If the separators ever collapsed, a bridge holding the read key
    /// could open the excerpt it is handed.
    #[test]
    fn the_status_key_is_neither_the_secret_nor_the_sealing_key() {
        let status = status_key_hex(SECRET);
        assert_ne!(status.as_bytes(), SECRET);
        assert_ne!(status, to_hex(&sealing_key(SECRET)));
    }

    #[test]
    fn a_different_secret_derives_a_different_status_key() {
        assert_ne!(status_key_hex(SECRET), status_key_hex(b"another-secret"));
    }

    #[test]
    fn a_different_secret_derives_a_different_key() {
        assert_ne!(sealing_key(SECRET), sealing_key(b"another-secret"));
    }

    #[test]
    fn the_debug_form_quotes_nothing() {
        let sealed = seal(SECRET, "a sentence nobody should see in a log").expect("sealing works");
        let shown = format!("{sealed:?}");

        assert_eq!(shown, "Sealed(..)");
        assert!(!shown.contains(&sealed.ciphertext_hex));
        assert!(!shown.contains("sentence"));
    }

    #[test]
    fn malformed_hex_is_refused_rather_than_guessed() {
        let sealed = seal(SECRET, "x").expect("sealing works");

        for broken in ["", "z", "abc", "zz"] {
            let attempt = Sealed {
                ciphertext_hex: broken.to_owned(),
                nonce_hex: sealed.nonce_hex.clone(),
            };
            assert_eq!(open(SECRET, &attempt), None);
        }

        // A nonce of the wrong length is refused before it reaches the cipher.
        let short = Sealed {
            ciphertext_hex: sealed.ciphertext_hex.clone(),
            nonce_hex: "00112233".to_owned(),
        };
        assert_eq!(open(SECRET, &short), None);
    }

    #[test]
    fn hex_round_trips_at_the_lengths_this_module_uses() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(from_hex(&to_hex(&bytes)).as_deref(), Some(bytes.as_slice()));
        assert_eq!(from_hex(&to_hex(&[])).as_deref(), Some(&[][..]));
        assert_eq!(from_hex("AABB").as_deref(), Some(&[0xaa, 0xbb][..]));
    }

    /// This module handles the one piece of session content allowed out; it must never be the
    /// place that sends it. The repository's outbound file count stays two.
    #[test]
    fn this_module_opens_no_outbound_connection() {
        let source = include_str!("crypt.rs");
        let code = source
            .split("#[cfg(test)]")
            .next()
            .expect("the file has a non-test part");

        for forbidden in ["reqwest", "lettre", "TcpStream", "TcpListener"] {
            assert!(
                !code.contains(forbidden),
                "crypt.rs must not reach the network, found {forbidden}"
            );
        }
    }

    fn flip_first_nibble(hex: &str) -> String {
        let mut out = hex.to_owned();
        let replacement = if hex.starts_with('a') { "b" } else { "a" };
        out.replace_range(0..1, replacement);
        out
    }
}
