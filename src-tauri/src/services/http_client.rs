//! Global HTTP clients with connection pooling.
//!
//! `reqwest::Client` natively supports connection pooling and multiplexing
//! multiple requests over the same TCP connection.  By creating a single
//! global client per API "profile" we avoid the overhead of creating a new
//! client (and TCP connection) for every request.
//!
//! Pool settings shared by all clients:
//! - `pool_max_idle_per_host(5)` – keep up to 5 idle connections per host
//! - `pool_idle_timeout(90s)` – close idle connections after 90 seconds
//! - `tcp_keepalive(60s)` – send TCP keep-alive probes every 60 seconds

use reqwest::header;
use std::sync::OnceLock;
use std::time::Duration;

/// Global general-purpose HTTP client (Yahoo Finance, exchange-rate APIs, etc.).
static GENERAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Global HTTP client configured specifically for the East Money API.
static EASTMONEY_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Return a shared general-purpose `reqwest::Client`.
///
/// The client is created once on first call and reused for the lifetime of the
/// process.  It is configured with a 15-second timeout, a simple `User-Agent`
/// header, and connection-pool settings suitable for moderate concurrency.
pub fn general_client() -> &'static reqwest::Client {
    GENERAL_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(5)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .user_agent("Mozilla/5.0")
            .build()
            .expect("failed to build general HTTP client")
    })
}

/// Return a shared `reqwest::Client` for East Money API requests.
///
/// In addition to the common pool settings this client is configured with:
/// - HTTP/1.1 only (avoids HTTP/2 negotiation issues with Chinese financial
///   servers)
/// - Browser-like default headers (`Referer`, `User-Agent`, `Accept`)
/// - 15-second request timeout
pub fn eastmoney_client() -> &'static reqwest::Client {
    EASTMONEY_CLIENT.get_or_init(|| {
        let mut default_headers = header::HeaderMap::new();
        default_headers.insert(
            header::REFERER,
            header::HeaderValue::from_static("https://www.eastmoney.com/"),
        );
        default_headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("*/*"),
        );
        default_headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            ),
        );
        reqwest::Client::builder()
            .timeout(Duration::from_secs(18))
            .pool_max_idle_per_host(5)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .default_headers(default_headers)
            .build()
            .expect("failed to build East Money HTTP client")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_general_client_returns_same_instance() {
        let c1 = general_client();
        let c2 = general_client();
        // OnceLock guarantees identity; both references point to the same
        // allocation.
        assert!(std::ptr::eq(c1, c2));
    }

    #[test]
    fn test_general_client_can_build_request() {
        let client = general_client();
        let req = client
            .get("https://example.com/test")
            .build()
            .expect("should build request");
        assert_eq!(req.method(), reqwest::Method::GET);
    }

    #[test]
    fn test_eastmoney_client_returns_same_instance() {
        let c1 = eastmoney_client();
        let c2 = eastmoney_client();
        assert!(std::ptr::eq(c1, c2));
    }

    #[test]
    fn test_eastmoney_client_can_build_request() {
        let client = eastmoney_client();
        let req = client
            .get("https://push2.eastmoney.com/test")
            .build()
            .expect("should build request");
        assert_eq!(req.method(), reqwest::Method::GET);
    }
}
