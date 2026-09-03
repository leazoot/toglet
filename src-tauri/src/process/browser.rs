//! Handing a sign-in URL to the user's own browser.
//!
//! Codex's app server returns an authorisation URL and leaves opening it to the client, so a
//! sign-in cannot happen without this. It is the one place Toglet asks the operating system to
//! launch something other than `codex app-server`.
//!
//! **The URL is never logged and never printed.** It carries the PKCE challenge and the OAuth
//! `state`, both of which the redaction rules forbid recording. It travels from the app server
//! straight into this function and no further.

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Opens `url` with whatever the user has registered as their browser.
///
/// Only `https://` is accepted. That is not decoration: the platform call resolves a registered
/// protocol handler, so a `file:` or a local path would ask the shell to open something on this
/// machine. Refusing anything else means a malformed or hostile server response cannot turn
/// into a launched program.
///
/// This is **not** a command line. The URL is passed as a single argument to a platform API;
/// there is no shell, no quoting and nothing to escape.
pub fn open_url(url: &str, phase: Phase) -> Result<()> {
    if !is_openable(url) {
        return Err(
            TogletError::new(ErrorCode::Internal, phase, false, UserAction::None)
                // The URL itself must not appear in the detail - it would be recorded.
                .with_detail("the sign-in address was not a plain https address"),
        );
    }
    open_platform(url, phase)
}

/// Whether this string is something safe to hand to the shell's URL handler.
///
/// Whitespace and control characters are rejected as well as the scheme: a URL carrying a
/// newline is not a URL, and refusing it here keeps anything odd from reaching the platform.
fn is_openable(url: &str) -> bool {
    const LIMIT: usize = 4096;

    url.len() <= LIMIT
        && url.starts_with("https://")
        && url.len() > "https://".len()
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[cfg(windows)]
fn open_platform(url: &str, phase: Phase) -> Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let target: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();

    // SAFETY: both strings are null-terminated and live until after the call returns. The
    // remaining arguments are null, which the API documents as "no parameters, no working
    // directory".
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // Documented contract: a value greater than 32 means the handler was launched. The value
    // itself is a legacy instance handle and means nothing else.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(
            TogletError::new(ErrorCode::Internal, phase, true, UserAction::Retry)
                .with_detail("no application is registered to open web addresses"),
        )
    }
}

#[cfg(not(windows))]
fn open_platform(_url: &str, phase: Phase) -> Result<()> {
    // macOS uses `open`, which is a different launch mechanism and needs its own verification
    // on a real machine. Failing loudly is better than a sign-in that silently
    // never shows the user anything.
    Err(
        TogletError::new(ErrorCode::Internal, phase, false, UserAction::None)
            .with_detail("opening a browser is not implemented on this platform"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_authorisation_url_is_accepted() {
        assert!(is_openable(
            "https://auth.openai.com/oauth/authorize?client_id=app_X&code_challenge=abc\
             &code_challenge_method=S256&redirect_uri=http%3A%2F%2Flocalhost%3A1455&state=xyz"
        ));
    }

    #[test]
    fn anything_that_is_not_https_is_refused() {
        // Each of these would ask the shell to open something local.
        assert!(!is_openable("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_openable("C:\\Windows\\System32\\calc.exe"));
        assert!(!is_openable("http://auth.openai.com/oauth/authorize"));
        assert!(!is_openable("ms-settings:privacy"));
        assert!(!is_openable("https://"));
        assert!(!is_openable(""));
    }

    #[test]
    fn a_url_carrying_whitespace_or_a_control_character_is_refused() {
        assert!(!is_openable("https://auth.openai.com/a b"));
        assert!(!is_openable("https://auth.openai.com/a\nb"));
        assert!(!is_openable("https://auth.openai.com/a\0b"));
    }

    #[test]
    fn an_absurdly_long_address_is_refused_rather_than_passed_on() {
        let long = format!("https://auth.openai.com/{}", "a".repeat(5000));

        assert!(!is_openable(&long));
    }

    #[test]
    fn open_url_refuses_before_it_reaches_the_platform() {
        // The point of the check: a rejected address must not become a launched program, so
        // this must fail on every platform, including the one that can open browsers.
        let error = open_url("file:///C:/Windows/System32/calc.exe", Phase::Login)
            .expect_err("a local path is not a sign-in address");

        assert_eq!(error.code(), ErrorCode::Internal);
    }
}
