//! Browser-facing request checks.
//!
//! The node signs whatever reaches `/api/publish`, so a web page in the user's
//! browser must not be able to reach it. Two checks close the usual routes:
//!
//! - **Host**: on a loopback listener, the `Host` header must name a loopback
//!   host. This defeats DNS rebinding, where an attacker's domain resolves to
//!   127.0.0.1 and the browser treats the node as same-origin with the attacker.
//! - **Origin**: a state-changing request that carries `Origin` must come from
//!   the node's own origin, and `Sec-Fetch-Site: cross-site` is refused. This
//!   defeats cross-site form posts, which need no CORS preflight.
//!
//! Non-browser clients (the CLI, the Python client, curl) send no `Origin` and
//! are unaffected.

use axum::extract::Request;
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::NodeError;

/// Hostname part of a `Host` header or URL authority, without port or brackets.
fn hostname(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    authority.rsplit_once(':').map_or(authority, |(h, _)| h)
}

/// True for `localhost`, `*.localhost`, 127.0.0.0/8, and `::1`.
pub fn is_loopback_host(host: &str) -> bool {
    let name = hostname(host).trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        return true;
    }
    name.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// Decide whether a request may proceed. `Err` carries the reason.
pub fn check(method: &Method, headers: &HeaderMap, loopback_only: bool) -> Result<(), String> {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if loopback_only {
        if let Some(host) = host {
            if !is_loopback_host(host) {
                return Err(format!("host {host:?} is not a loopback name"));
            }
        }
    }

    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|site| site.eq_ignore_ascii_case("cross-site"))
    {
        return Err("cross-site request refused".into());
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let origin = origin
        .to_str()
        .map_err(|_| "unreadable Origin".to_owned())?;
    let authority = origin
        .split_once("://")
        .map(|(_, rest)| rest.trim_end_matches('/'))
        .ok_or_else(|| format!("origin {origin:?} refused"))?;
    match host {
        Some(host) if authority.eq_ignore_ascii_case(host) => Ok(()),
        _ => Err(format!("origin {origin:?} refused")),
    }
}

/// Router middleware applying [`check`].
pub async fn layer(loopback_only: bool, request: Request, next: Next) -> Response {
    match check(request.method(), request.headers(), loopback_only) {
        Ok(()) => next.run(request).await,
        Err(reason) => {
            tracing::warn!(%reason, path = %request.uri().path(), "refused request");
            NodeError::Rejected(StatusCode::FORBIDDEN, reason).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn loopback_names() {
        for host in [
            "localhost",
            "localhost:4780",
            "app.localhost:1",
            "127.0.0.1:4780",
            "127.8.9.1",
            "[::1]:4780",
            "LOCALHOST.",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "evil.com",
            "127.0.0.1.evil.com",
            "localhost.evil.com",
            "[::2]:1",
            "10.0.0.1",
        ] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn rebinding_is_refused_on_loopback_only() {
        let h = headers(&[("host", "attacker.example:4780")]);
        assert!(check(&Method::GET, &h, true).is_err());
        assert!(check(&Method::GET, &h, false).is_ok());
    }

    #[test]
    fn cross_origin_writes_are_refused() {
        let h = headers(&[
            ("host", "127.0.0.1:4780"),
            ("origin", "https://evil.example"),
        ]);
        assert!(check(&Method::POST, &h, true).is_err());
        assert!(check(&Method::GET, &h, true).is_ok());

        let h = headers(&[("host", "127.0.0.1:4780"), ("sec-fetch-site", "cross-site")]);
        assert!(check(&Method::POST, &h, true).is_err());

        let h = headers(&[("host", "127.0.0.1:4780"), ("origin", "null")]);
        assert!(check(&Method::POST, &h, true).is_err());
    }

    #[test]
    fn same_origin_and_non_browser_writes_pass() {
        let h = headers(&[
            ("host", "127.0.0.1:4780"),
            ("origin", "http://127.0.0.1:4780"),
        ]);
        assert!(check(&Method::POST, &h, true).is_ok());
        // Vite dev proxy forwards the dev server's Host and Origin unchanged.
        let h = headers(&[
            ("host", "localhost:5173"),
            ("origin", "http://localhost:5173"),
        ]);
        assert!(check(&Method::POST, &h, true).is_ok());
        let h = headers(&[("host", "127.0.0.1:4780")]);
        assert!(check(&Method::POST, &h, true).is_ok());
        assert!(check(&Method::POST, &HeaderMap::new(), true).is_ok());
    }
}
