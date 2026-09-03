//! Redaction of secrets and quasi-sensitive values.
//!
//! Redaction must happen *before* a value reaches a log sink or an error message — never as
//! a post-processing filter. Every diagnostic string therefore passes through [`redact`] at
//! construction time.
//!
//! Implemented as a hand-written scanner rather than with a regex engine: the patterns are
//! few and anchored, a scanner has no backtracking behaviour to reason about, and it keeps
//! the dependency surface of a credential-handling binary smaller.

const TOKEN_PLACEHOLDER: &str = "[redacted:token]";
const EMAIL_PLACEHOLDER: &str = "[redacted:email]";
const PATH_PLACEHOLDER: &str = "[redacted:path]";
const URL_PLACEHOLDER: &str = "[redacted:url]";

/// Query/parameter names whose *values* are secrets or can be used to complete a login.
const SENSITIVE_PARAMS: &[&str] = &[
    "access_token",
    "refresh_token",
    "id_token",
    "client_secret",
    "code_verifier",
    "code_challenge",
    "redirect_uri",
    "code",
    "state",
];

/// POSIX roots that identify an absolute path worth hiding. A bare `/` is deliberately not
/// treated as a path start: ordinary prose contains slashes and over-redaction would make
/// diagnostics useless.
const POSIX_PATH_ROOTS: &[&str] = &[
    "/Users/",
    "/home/",
    "/private/",
    "/var/",
    "/tmp/",
    "/Applications/",
    "/Library/",
];

/// Removes secrets and quasi-sensitive values from `input`.
///
/// Covers, in this order: whole `http`/`https` URLs, OAuth/token parameter values, JWT-shaped
/// tokens, `sk-` style API keys, e-mail addresses, and absolute filesystem paths. E-mails are
/// removed outright rather than masked — logs are expected to identify accounts by their random
/// internal id.
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Checked before the parameter matcher so a whole callback URL goes at once. Stripping
        // only `code=` and `state=` leaves the scheme, host and path standing, and an auth host
        // can identify the organisation an account belongs to - the same class of disclosure as
        // an absolute path. Found by the leak probe, not by review.
        if let Some(end) = match_url(input, i) {
            out.push_str(URL_PLACEHOLDER);
            i = end;
        } else if let Some(end) = match_param_value(input, i) {
            out.push_str(&input[i..end.value_start]);
            out.push_str(TOKEN_PLACEHOLDER);
            i = end.value_end;
        } else if let Some(end) = match_jwt(input, i) {
            out.push_str(TOKEN_PLACEHOLDER);
            i = end;
        } else if let Some(end) = match_api_key(input, i) {
            out.push_str(TOKEN_PLACEHOLDER);
            i = end;
        } else if let Some((start, end)) = match_email(input, i) {
            // The local part was already copied into `out`; drop it back off.
            out.truncate(out.len() - (i - start));
            out.push_str(EMAIL_PLACEHOLDER);
            i = end;
        } else if let Some(end) = match_path(input, i) {
            out.push_str(PATH_PLACEHOLDER);
            i = end;
        } else {
            let ch_len = char_len(bytes[i]);
            out.push_str(&input[i..i + ch_len]);
            i += ch_len;
        }
    }

    out
}

struct ParamMatch {
    value_start: usize,
    value_end: usize,
}

/// An `http`/`https` URL, up to the first whitespace.
///
/// Removed whole rather than piecemeal. A URL carries a host, a path and a query, and each can
/// disclose something: the host names the identity provider, the query carries the OAuth `code`
/// and `state`. Keeping any part of it would mean arguing about which parts are safe.
fn match_url(input: &str, at: usize) -> Option<usize> {
    let rest = input.get(at..)?;
    let scheme_len = if rest.starts_with("https://") {
        "https://".len()
    } else if rest.starts_with("http://") {
        "http://".len()
    } else {
        return None;
    };
    // A scheme with nothing after it is not a URL worth redacting.
    let after_scheme = rest.get(scheme_len..)?;
    if after_scheme.is_empty() || after_scheme.starts_with(char::is_whitespace) {
        return None;
    }
    let end = after_scheme
        .find(char::is_whitespace)
        .map_or(rest.len(), |offset| scheme_len + offset);
    Some(at + end)
}

/// Matches `name=value` where `name` is sensitive. Returns the span of `value`.
fn match_param_value(input: &str, at: usize) -> Option<ParamMatch> {
    let rest = &input[at..];
    for name in SENSITIVE_PARAMS {
        let Some(after_name) = rest.strip_prefix(name) else {
            continue;
        };
        let Some(after_eq) = after_name.strip_prefix('=') else {
            continue;
        };
        // Only a parameter if the preceding byte cannot be part of a longer identifier;
        // this stops `encoded_state=` from matching `state=`.
        if at > 0 && is_ident_byte(input.as_bytes()[at - 1]) {
            continue;
        }
        let value_start = at + name.len() + 1;
        let value_len = after_eq
            .find(|c: char| c == '&' || c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(after_eq.len());
        if value_len == 0 {
            continue;
        }
        return Some(ParamMatch {
            value_start,
            value_end: value_start + value_len,
        });
    }
    None
}

/// Matches a JWT-shaped token: the `eyJ` header prefix followed by base64url data.
fn match_jwt(input: &str, at: usize) -> Option<usize> {
    if !input[at..].starts_with("eyJ") {
        return None;
    }
    if at > 0 && is_ident_byte(input.as_bytes()[at - 1]) {
        return None;
    }
    let end = scan_while(input, at, |b| is_base64url_byte(b) || b == b'.');
    // Short `eyJ...` runs are ordinary words, not credentials.
    if end - at < 16 { None } else { Some(end) }
}

/// Matches `sk-` style API keys.
fn match_api_key(input: &str, at: usize) -> Option<usize> {
    if !input[at..].starts_with("sk-") {
        return None;
    }
    if at > 0 && is_ident_byte(input.as_bytes()[at - 1]) {
        return None;
    }
    let end = scan_while(input, at + 3, is_base64url_byte);
    if end - at < 16 { None } else { Some(end) }
}

/// Matches an e-mail address. `at` is expected to sit on the `@`; the local part is located
/// by scanning backwards, so the caller must undo the bytes it already emitted.
fn match_email(input: &str, at: usize) -> Option<(usize, usize)> {
    let bytes = input.as_bytes();
    if bytes[at] != b'@' {
        return None;
    }

    let mut start = at;
    while start > 0 && is_email_local_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == at {
        return None;
    }

    let end = scan_while(input, at + 1, is_email_domain_byte);
    // A domain must contain a dot with at least one character on either side.
    let domain = &input[at + 1..end];
    match domain.find('.') {
        Some(dot) if dot > 0 && dot + 1 < domain.len() => Some((start, end)),
        _ => None,
    }
}

/// Matches an absolute filesystem path (Windows drive, UNC, or a known POSIX root).
fn match_path(input: &str, at: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let rest = &input[at..];

    let is_windows_drive = bytes.len() > at + 2
        && bytes[at].is_ascii_alphabetic()
        && bytes[at + 1] == b':'
        && (bytes[at + 2] == b'\\' || bytes[at + 2] == b'/')
        && (at == 0 || !is_ident_byte(bytes[at - 1]));
    let is_unc = rest.starts_with(r"\\");
    let is_posix = POSIX_PATH_ROOTS.iter().any(|root| rest.starts_with(root));

    if !(is_windows_drive || is_unc || is_posix) {
        return None;
    }

    Some(scan_while(input, at, |b| {
        !(b as char).is_whitespace() && b != b'"' && b != b'\'' && b != b','
    }))
}

fn scan_while(input: &str, from: usize, pred: impl Fn(u8) -> bool) -> usize {
    let bytes = input.as_bytes();
    let mut i = from;
    while i < bytes.len() && pred(bytes[i]) {
        i += 1;
    }
    i
}

/// UTF-8 sequence length from a leading byte. Keeps the scanner from splitting a character
/// when it copies unmatched input through.
fn char_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_base64url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn is_email_local_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn is_email_domain_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_jwt_shaped_token() {
        let input = "auth failed for eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U end";
        let out = redact(input);
        assert_eq!(out, format!("auth failed for {TOKEN_PLACEHOLDER} end"));
    }

    #[test]
    fn removes_api_key() {
        let out = redact("key sk-abcdefghijklmnopqrstuvwxyz012345 done");
        assert_eq!(out, format!("key {TOKEN_PLACEHOLDER} done"));
    }

    #[test]
    fn removes_email_entirely_including_domain() {
        let out = redact("account lea.smith+codex@gmail.com refreshed");
        assert_eq!(out, format!("account {EMAIL_PLACEHOLDER} refreshed"));
    }

    #[test]
    fn removes_windows_absolute_path() {
        let out = redact(r"wrote C:\Users\someone\.codex\auth.json ok");
        assert_eq!(out, format!("wrote {PATH_PLACEHOLDER} ok"));
    }

    #[test]
    fn removes_unc_path() {
        let out = redact(r"target \\server\share\auth.json");
        assert_eq!(out, format!("target {PATH_PLACEHOLDER}"));
    }

    #[test]
    fn removes_posix_home_path() {
        let out = redact("reading /Users/someone/.codex/auth.json now");
        assert_eq!(out, format!("reading {PATH_PLACEHOLDER} now"));
    }

    #[test]
    fn removes_oauth_callback_parameters() {
        let out = redact("callback code=4/0AY0e-g7l8Qx&state=abc123def456&ok=1");
        assert_eq!(
            out,
            format!("callback code={TOKEN_PLACEHOLDER}&state={TOKEN_PLACEHOLDER}&ok=1")
        );
    }

    #[test]
    fn keeps_non_sensitive_text_unchanged() {
        let input = "switch phase=verify code=  account_id=a1b2c3 retryable=false";
        assert_eq!(redact(input), input);
    }

    #[test]
    fn removes_a_callback_url_whole_including_its_host() {
        let out = redact("opened https://auth.example.com/cb?code=4/0AY0e-g7&state=xyz789 ok");

        assert_eq!(out, format!("opened {URL_PLACEHOLDER} ok"));
        assert!(!out.contains("example.com"));
    }

    #[test]
    fn removes_a_url_at_the_end_of_a_line() {
        assert_eq!(
            redact("see http://localhost:1455/auth/callback?code=abc"),
            format!("see {URL_PLACEHOLDER}")
        );
    }

    #[test]
    fn leaves_a_bare_scheme_word_alone() {
        assert_eq!(redact("use https:// for tls"), "use https:// for tls");
    }

    #[test]
    fn does_not_match_parameter_name_suffixes() {
        let input = "encoded_state=plainvalue";
        assert_eq!(redact(input), input);
    }

    #[test]
    fn leaves_short_ey_words_alone() {
        let input = "eyJab is too short";
        assert_eq!(redact(input), input);
    }

    #[test]
    fn does_not_treat_bare_slash_as_a_path() {
        let input = "ratio 3/4 and a/b";
        assert_eq!(redact(input), input);
    }

    #[test]
    fn preserves_multibyte_characters() {
        let input = "切换失败：阶段 verify";
        assert_eq!(redact(input), input);
    }

    #[test]
    fn redacts_every_secret_in_a_mixed_line() {
        let out = redact(
            r"user a@b.com path C:\Users\x\auth.json token eyJhbGciOiJIUzI1NiJ9.payloadpayload",
        );
        assert!(!out.contains("a@b.com"));
        assert!(!out.contains(r"C:\Users"));
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"));
    }
}
