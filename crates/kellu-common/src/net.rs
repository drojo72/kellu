//! Endpoint URL rules shared by configuration validation and the HTTP transport.
//!
//! The URL is parsed with the `http` crate's `Uri`, the same parser the HTTP client uses, so
//! what is validated is what is connected to. A WHATWG parser such as the `url` crate reads some
//! inputs differently (for example backslashes), which would open a parser differential.

use http::Uri;

const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Return why `url` is not an acceptable provider endpoint, or `None` if it is.
///
/// Endpoints must be absolute http(s) URLs without embedded credentials. Plain http is accepted
/// only for localhost (local model runtimes); everything else must use https.
#[must_use]
pub fn endpoint_problem(url: &str) -> Option<&'static str> {
    let Ok(uri) = url.parse::<Uri>() else {
        return Some("invalid URL");
    };
    let (Some(scheme), Some(authority)) = (uri.scheme_str(), uri.authority()) else {
        return Some("expected an absolute http(s) URL");
    };
    let is_http = scheme.eq_ignore_ascii_case("http");
    if !is_http && !scheme.eq_ignore_ascii_case("https") {
        return Some("expected an absolute http(s) URL");
    }
    if authority.as_str().contains('@') {
        return Some("credentials in URLs are not allowed");
    }
    if is_http
        && !LOCAL_HOSTS
            .iter()
            .any(|local| authority.host().eq_ignore_ascii_case(local))
    {
        return Some("plain http is only allowed for localhost; use https");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_endpoints_are_accepted() {
        for url in [
            "https://api.example.com/v1",
            "https://api.example.com:8443/v1/",
            "HTTPS://Example.com",
        ] {
            assert_eq!(endpoint_problem(url), None, "{url}");
        }
    }

    #[test]
    fn plain_http_is_only_allowed_for_localhost() {
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:8080",
            "http://[::1]:8080/v1",
            "http://LOCALHOST/v1",
        ] {
            assert_eq!(endpoint_problem(url), None, "{url}");
        }
        for url in [
            "http://example.com/v1",
            "http://localhost.evil.com/v1",
            "http://127.0.0.1.evil.com",
        ] {
            assert!(
                endpoint_problem(url).is_some_and(|p| p.contains("plain http")),
                "{url}"
            );
        }
    }

    #[test]
    fn credentials_in_urls_are_rejected() {
        for url in [
            "https://user:pass@example.com/v1",
            "https://user@example.com",
            "http://localhost@evil.com/",
            "http://localhost:80@evil.com/",
        ] {
            assert_eq!(
                endpoint_problem(url),
                Some("credentials in URLs are not allowed"),
                "{url}"
            );
        }
    }

    #[test]
    fn non_http_schemes_and_relative_urls_are_rejected() {
        for url in [
            "ftp://example.com",
            "file:///etc/passwd",
            "example.com/v1",
            "/v1/chat",
            "//example.com",
        ] {
            assert!(endpoint_problem(url).is_some(), "{url}");
        }
    }

    #[test]
    fn malformed_urls_fail_closed() {
        for url in [
            "",
            "https://",
            "https://exa mple.com",
            "https://example.com\\@evil.com",
            "https://a\nb.com",
        ] {
            assert!(endpoint_problem(url).is_some(), "{url:?}");
        }
    }
}
