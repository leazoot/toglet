//! Hands a sign-in URL to the user's browser.
//!
//! The URL carries the PKCE challenge and the OAuth `state`, so it is never logged or printed.

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Opens `url` with the user's registered browser.
///
/// Only `https://` is accepted: the platform call resolves protocol handlers, so a `file:` URL
/// or a local path would launch something on this machine. The URL goes to a platform API, not
/// a shell.
pub fn open_url(url: &str, phase: Phase) -> Result<()> {
    if !is_openable(url) {
        return Err(
            TogletError::new(ErrorCode::Internal, phase, false, UserAction::None)
                // The URL itself must not appear in the detail, which is logged.
                .with_detail("the sign-in address was not a plain https address"),
        );
    }
    open_platform(url, phase)
}

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

    // SAFETY: both strings are null-terminated and outlive the call; the null arguments mean no
    // parameters and no working directory.
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

    // Documented contract: a value greater than 32 means the handler was launched.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(
            TogletError::new(ErrorCode::Internal, phase, true, UserAction::Retry)
                .with_detail("no application is registered to open web addresses"),
        )
    }
}

/// Uses Launch Services rather than the `open` command, so there is no subprocess or shell.
#[cfg(not(windows))]
fn open_platform(url: &str, phase: Phase) -> Result<()> {
    use std::ffi::c_void;

    type CFTypeRef = *const c_void;
    type CFURLRef = *const c_void;
    type OSStatus = i32;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const NO_ERR: OSStatus = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            base_url: CFURLRef,
        ) -> CFURLRef;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSOpenCFURLRef(url: CFURLRef, launched: *mut CFURLRef) -> OSStatus;
    }

    // Cannot fail after `is_openable` capped the length, but is still no reason to panic mid
    // sign-in.
    let Ok(length) = isize::try_from(url.len()) else {
        return Err(refused(phase));
    };
    // SAFETY: `url` outlives the call, which reads exactly `length` bytes; a null allocator means
    // the default one and a null base means an absolute URL.
    let cf_url = unsafe {
        CFURLCreateWithBytes(
            std::ptr::null(),
            url.as_ptr(),
            length,
            K_CF_STRING_ENCODING_UTF8,
            std::ptr::null(),
        )
    };
    if cf_url.is_null() {
        return Err(refused(phase));
    }
    // SAFETY: `cf_url` is owned here and released exactly once; a null out-pointer means the
    // launched application is not wanted.
    let status = unsafe {
        let status = LSOpenCFURLRef(cf_url, std::ptr::null_mut());
        CFRelease(cf_url);
        status
    };
    if status == NO_ERR {
        Ok(())
    } else {
        Err(
            TogletError::new(ErrorCode::Internal, phase, true, UserAction::Retry)
                .with_detail("no application is registered to open web addresses"),
        )
    }
}

#[cfg(not(windows))]
fn refused(phase: Phase) -> TogletError {
    TogletError::new(ErrorCode::Internal, phase, false, UserAction::None)
        // The URL itself must not appear in the detail, which is logged.
        .with_detail("the sign-in address could not be handed to the browser")
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
        // Must fail on every platform, including those that can open browsers.
        let error = open_url("file:///C:/Windows/System32/calc.exe", Phase::Login)
            .expect_err("a local path is not a sign-in address");

        assert_eq!(error.code(), ErrorCode::Internal);
    }
}
