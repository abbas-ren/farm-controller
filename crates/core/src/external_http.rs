//! Shared outbound HTTP client policy for external integrations.
//!
//! All FarmController crates that call TestRail, QMetry, Jira, GitLab, or
//! similar services should build clients through these helpers so timeouts,
//! redirect policy, and configured-URL validation stay consistent. Replaces
//! ad-hoc Axios setups from the old Node services.

use std::time::Duration;

use reqwest::{Client, ClientBuilder, Url, redirect::Policy};

/// Hard cap on TCP connect time even when the request timeout is larger.
const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Suggested retry budget for explicit operator-triggered external commands.
pub const EXPLICIT_COMMAND_ATTEMPTS: usize = 3;

/// PURPOSE: Build a reqwest client with farm-wide timeout and no redirects.
/// USE: Integrations call this (or [`client_builder`]) when constructing
/// TestRail/QMetry/Jira/etc. clients from config.
pub fn client(request_timeout: Duration) -> Result<Client, reqwest::Error> {
    client_builder(request_timeout).build()
}

/// PURPOSE: Shared `ClientBuilder` defaults (connect/request timeouts, no redirects).
/// USE: Prefer when callers need to add default headers (e.g. Bearer tokens)
/// before `.build()`.
pub fn client_builder(request_timeout: Duration) -> ClientBuilder {
    Client::builder()
        // Cap connect separately so slow DNS cannot exceed 5s even if timeout is high.
        .connect_timeout(request_timeout.min(MAX_CONNECT_TIMEOUT))
        .timeout(request_timeout)
        // Never follow redirects — prevents open-redirect / SSRF chaining.
        .redirect(Policy::none())
}

/// PURPOSE: Validate a configured base URL for scheme, host, and no embedded creds.
/// USE: Invoked from [`crate::config::AppConfig::validate`] for every outbound
/// base URL. Empty values are allowed (feature disabled); non-empty must be
/// `http`/`https` without userinfo.
pub fn validate_configured_url(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    let url = Url::parse(value).map_err(|_| format!("{name} must be a valid HTTP(S) URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(format!("{name} must be a valid HTTP(S) URL"));
    }
    // Credentials belong in headers/config secrets, not in the URL string.
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} must not contain embedded credentials"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Unit tests for URL policy and client timeout/redirect behavior.

    use std::{convert::Infallible, time::Instant};

    use axum::{Router, routing::get};
    use tokio::net::TcpListener;

    use super::*;

    /// PURPOSE: Reject non-HTTP schemes and userinfo in configured URLs.
    #[test]
    fn configured_urls_require_http_without_credentials() {
        assert!(validate_configured_url("service.url", "https://service.example/api").is_ok());
        assert!(validate_configured_url("service.url", "").is_ok());
        assert!(validate_configured_url("service.url", "file:///etc/passwd").is_err());
        assert!(validate_configured_url("service.url", "https://user:pass@example.com").is_err());
    }

    /// PURPOSE: Prove redirects are not followed and request timeout is enforced.
    #[tokio::test]
    async fn client_refuses_redirects_and_bounds_requests() {
        let app = Router::new()
            .route(
                "/redirect",
                get(|| async { axum::response::Redirect::temporary("/target") }),
            )
            .route("/target", get(|| async { "followed" }))
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Result::<_, Infallible>::Ok("late")
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = client(Duration::from_millis(20)).unwrap();

        let redirected = client
            .get(format!("http://{address}/redirect"))
            .send()
            .await
            .unwrap();
        assert!(redirected.status().is_redirection());
        assert_eq!(redirected.url().path(), "/redirect");

        let started = Instant::now();
        assert!(
            client
                .get(format!("http://{address}/slow"))
                .send()
                .await
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_millis(90));
        server.abort();
    }
}
