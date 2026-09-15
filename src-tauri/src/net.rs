//! Shape checks for user-typed outbound addresses.
//!
//! Shared by `notify::send` and `remote::poll` so the security check exists only once.
//! Callers turn a refusal into their own error.

/// Long enough for any real endpoint, short enough that an address cannot become a payload.
pub const MAX_URL_LEN: usize = 512;

/// The host part of an address, without the port or the credentials some addresses carry.
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // `user:password@host` is legal in an address and the part before the `@` is a credential.
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() { None } else { Some(host) }
}

/// Whether an authority (an address with its scheme already removed) names this machine.
pub fn is_loopback_authority(rest: &str) -> bool {
    host_of(&format!("http://{rest}")).is_some_and(|host| {
        host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1"
    })
}

/// Whether Toglet may send to this address at all.
///
/// `https` only, except plain `http` to loopback, where a local service has no certificate.
/// Control characters are refused: a CR/LF in an address is an attempt to split the request.
pub fn is_safe_endpoint(url: &str) -> bool {
    let Some(rest) = (match url.strip_prefix("https://") {
        Some(rest) => Some(rest),
        None => url
            .strip_prefix("http://")
            .filter(|rest| is_loopback_authority(rest)),
    }) else {
        return false;
    };

    !rest.is_empty()
        && url.len() <= MAX_URL_LEN
        && url.is_ascii()
        && !url.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
        && !rest.starts_with('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_address_is_refused_unless_it_is_this_machine() {
        assert!(is_safe_endpoint("https://example.com/hook"));
        assert!(is_safe_endpoint("http://127.0.0.1:8080/poll"));
        assert!(is_safe_endpoint("http://localhost/poll"));
        assert!(!is_safe_endpoint("http://example.com/hook"));
        assert!(!is_safe_endpoint("ftp://example.com"));
        assert!(!is_safe_endpoint("example.com"));
    }

    #[test]
    fn an_address_that_could_split_a_request_is_refused() {
        assert!(!is_safe_endpoint("https://example.com/a\r\nHost: evil"));
        assert!(!is_safe_endpoint("https://example.com/a b"));
        assert!(!is_safe_endpoint("https://exämple.com/a"));
    }

    #[test]
    fn an_address_with_no_host_is_refused() {
        assert!(!is_safe_endpoint("https://"));
        assert!(!is_safe_endpoint("https:///path"));
    }

    #[test]
    fn an_address_longer_than_the_cap_is_refused() {
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(!is_safe_endpoint(&long));
    }

    #[test]
    fn the_host_is_read_without_the_port_or_the_credentials() {
        assert_eq!(
            host_of("https://user:pw@example.com:8443/x"),
            Some("example.com")
        );
        assert_eq!(host_of("https://example.com"), Some("example.com"));
        assert_eq!(host_of("https://"), None);
    }
}
