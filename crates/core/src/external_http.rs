use std::time::Duration;

use reqwest::{Client, ClientBuilder, Url, redirect::Policy};

const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const EXPLICIT_COMMAND_ATTEMPTS: usize = 3;

pub fn client(request_timeout: Duration) -> Result<Client, reqwest::Error> {
    client_builder(request_timeout).build()
}

pub fn client_builder(request_timeout: Duration) -> ClientBuilder {
    Client::builder()
        .connect_timeout(request_timeout.min(MAX_CONNECT_TIMEOUT))
        .timeout(request_timeout)
        .redirect(Policy::none())
}

pub fn validate_configured_url(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    let url = Url::parse(value).map_err(|_| format!("{name} must be a valid HTTP(S) URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(format!("{name} must be a valid HTTP(S) URL"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} must not contain embedded credentials"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, time::Instant};

    use axum::{Router, routing::get};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn configured_urls_require_http_without_credentials() {
        assert!(validate_configured_url("service.url", "https://service.example/api").is_ok());
        assert!(validate_configured_url("service.url", "").is_ok());
        assert!(validate_configured_url("service.url", "file:///etc/passwd").is_err());
        assert!(validate_configured_url("service.url", "https://user:pass@example.com").is_err());
    }

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
