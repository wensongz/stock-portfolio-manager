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

#[cfg(target_os = "macos")]
mod macos_proxy;

#[cfg(any(target_os = "macos", test))]
fn has_proxy_environment(scheme: &str, lookup: impl Fn(&str) -> Option<String>) -> bool {
    let protocol_keys = match scheme {
        "https" => ["HTTPS_PROXY", "https_proxy"],
        "http" => ["HTTP_PROXY", "http_proxy"],
        _ => return false,
    };
    protocol_keys
        .into_iter()
        .chain(["ALL_PROXY", "all_proxy"])
        .any(|key| lookup(key).is_some_and(|value| !value.is_empty()))
}

#[cfg(target_os = "macos")]
fn matches_no_proxy(target: &reqwest::Url, no_proxy: &str) -> bool {
    let Ok(uri) = target.as_str().parse() else {
        return false;
    };
    // Reuse reqwest's underlying matcher for domains, IP ranges and IPv6.
    // This placeholder is never contacted; it only enables bypass matching.
    hyper_util::client::proxy::matcher::Matcher::builder()
        .all("http://proxy.invalid")
        .no(no_proxy)
        .build()
        .intercept(&uri)
        .is_none()
}

/// Global general-purpose HTTP client (Yahoo Finance, exchange-rate APIs, etc.).
static GENERAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Global HTTP client configured specifically for the East Money API.
static EASTMONEY_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Global HTTP client configured specifically for the Xueqiu (雪球) API.
/// Built with `cookie_store(true)` so that the `xq_a_token` session cookie
/// obtained from the Xueqiu homepage is automatically stored and sent on
/// subsequent API requests.
static XUEQIU_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static XUEQIU_COOKIE_JAR: OnceLock<std::sync::Arc<reqwest::cookie::Jar>> = OnceLock::new();

pub struct XueqiuRequest {
    requests: Vec<reqwest::RequestBuilder>,
}

impl XueqiuRequest {
    pub fn header(mut self, name: header::HeaderName, value: impl AsRef<str>) -> Self {
        self.requests = self
            .requests
            .into_iter()
            .map(|request| request.header(name.clone(), value.as_ref()))
            .collect();
        self
    }

    pub async fn send(self) -> Result<reqwest::Response, reqwest::Error> {
        let mut requests = self.requests.into_iter();
        let mut request = requests.next().expect("at least one proxy route");
        loop {
            match request.send().await {
                Err(error) if error.is_connect() || error.is_timeout() => {
                    if let Some(next) = requests.next() {
                        request = next;
                    } else {
                        return Err(error);
                    }
                }
                result => return result,
            }
        }
    }
}

#[cfg(target_os = "macos")]
static XUEQIU_PROXY_CLIENTS: OnceLock<
    std::sync::Mutex<std::collections::HashMap<macos_proxy::ProxyRoute, reqwest::Client>>,
> = OnceLock::new();

/// Global HTTP client for AI provider requests (OpenAI-compatible chat APIs).
///
/// LLM streaming responses can take minutes to finish, so this client uses a
/// generous 5-minute timeout (vs the 15s general client). No `User-Agent` is
/// set because provider APIs (OpenAI / GLM / Kimi / etc.) do not require one.
static AI_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

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

/// Return a shared `reqwest::Client` for Xueqiu (雪球) API requests.
///
/// The client is created once on first call and reused for the lifetime of
/// the process.  It is configured with:
/// - Cookie store enabled (so `xq_a_token` is persisted across requests)
/// - Browser-like default headers (`Referer`, `User-Agent`, `Accept`)
/// - 15-second request timeout
pub fn xueqiu_client() -> &'static reqwest::Client {
    XUEQIU_CLIENT.get_or_init(|| {
        xueqiu_client_builder()
            .build()
            .expect("failed to build Xueqiu HTTP client")
    })
}

fn xueqiu_client_builder() -> reqwest::ClientBuilder {
    let mut default_headers = header::HeaderMap::new();
    default_headers.insert(
        header::REFERER,
        header::HeaderValue::from_static("https://xueqiu.com/"),
    );
    default_headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json, text/plain, */*"),
    );
    default_headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            ),
        );
    default_headers.insert(
        "X-Requested-With",
        header::HeaderValue::from_static("XMLHttpRequest"),
    );
    let default_redirects = reqwest::redirect::Policy::default();
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.url().host_str() == Some("check.xueqiu.com") {
                attempt.stop()
            } else {
                default_redirects.redirect(attempt)
            }
        }))
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(5)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .cookie_provider(XUEQIU_COOKIE_JAR.get_or_init(Default::default).clone())
        .default_headers(default_headers)
}

/// Resolve native PAC on a blocking worker before building each request.
/// reqwest's custom-proxy callback is synchronous and cannot report native
/// resolution failures. Keep that work off the asynchronous executor.
pub async fn xueqiu_get(url: &str) -> Result<XueqiuRequest, String> {
    #[cfg(target_os = "macos")]
    {
        let target =
            reqwest::Url::parse(url).map_err(|error| format!("Invalid Xueqiu URL: {error}"))?;
        let no_proxy = std::env::var("NO_PROXY")
            .or_else(|_| std::env::var("no_proxy"))
            .unwrap_or_default();
        if matches_no_proxy(&target, &no_proxy) {
            return Ok(XueqiuRequest {
                requests: vec![xueqiu_client_for_route(macos_proxy::ProxyRoute::Direct)?.get(url)],
            });
        }
        if !has_proxy_environment(target.scheme(), |key| std::env::var(key).ok()) {
            let routes = tokio::task::spawn_blocking(move || macos_proxy::resolve(&target))
                .await
                .map_err(|error| format!("Xueqiu system proxy resolution failed: {error}"))?
                .map_err(|error| format!("Xueqiu system proxy resolution failed: {error}"))?;
            let requests = routes
                .into_iter()
                .map(|route| xueqiu_client_for_route(route).map(|client| client.get(url)))
                .collect::<Result<_, _>>()?;
            return Ok(XueqiuRequest { requests });
        }
    }
    Ok(XueqiuRequest {
        requests: vec![xueqiu_client().get(url)],
    })
}

#[cfg(target_os = "macos")]
fn xueqiu_client_for_route(route: macos_proxy::ProxyRoute) -> Result<reqwest::Client, String> {
    let clients = XUEQIU_PROXY_CLIENTS.get_or_init(Default::default);
    let mut clients = clients.lock().unwrap();
    if let Some(client) = clients.get(&route) {
        return Ok(client.clone());
    }
    let client = xueqiu_route_builder(&route, reqwest::NoProxy::from_env())?
        .build()
        .map_err(|error| format!("Failed to build Xueqiu proxy client: {error}"))?;
    clients.insert(route, client.clone());
    Ok(client)
}

#[cfg(target_os = "macos")]
fn xueqiu_route_builder(
    route: &macos_proxy::ProxyRoute,
    no_proxy: Option<reqwest::NoProxy>,
) -> Result<reqwest::ClientBuilder, String> {
    let builder = xueqiu_client_builder().no_proxy();
    Ok(match route {
        macos_proxy::ProxyRoute::Direct => builder,
        macos_proxy::ProxyRoute::Proxy(url) => builder.proxy(
            reqwest::Proxy::all(url)
                .map_err(|error| format!("Invalid Xueqiu system proxy: {error}"))?
                .no_proxy(no_proxy),
        ),
    })
}

/// Return a shared `reqwest::Client` for AI provider API requests.
///
/// Configured with a 5-minute timeout to accommodate slow streaming chat
/// completions, and the same connection-pool settings as the other clients.
pub fn ai_client() -> &'static reqwest::Client {
    AI_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(5)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .expect("failed to build AI HTTP client")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn no_proxy_matching_can_bypass_unavailable_pac_before_resolution() {
        for (target, bypass, expected) in [
            ("https://stock.xueqiu.com/", "xueqiu.com", true),
            ("https://stock.xueqiu.com/", "example.com", false),
            ("https://xueqiu.com.example/", "xueqiu.com", false),
            ("http://192.0.2.10/", "192.0.2.0/24", true),
            ("http://[::1]/", "::1", true),
            ("https://stock.xueqiu.com/", "*", true),
        ] {
            assert_eq!(
                matches_no_proxy(&reqwest::Url::parse(target).unwrap(), bypass),
                expected,
                "{target}, {bypass}"
            );
        }
    }

    #[test]
    fn explicit_proxy_environment_is_checked_for_the_request_protocol() {
        let vars = [
            ("HTTP_PROXY", "http://http-proxy:3128"),
            ("https_proxy", "http://https-proxy:3128"),
        ];
        let lookup = |key: &str| {
            vars.iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        };
        assert!(has_proxy_environment("https", lookup));
        assert!(has_proxy_environment("http", lookup));
        assert!(!has_proxy_environment("https", |key| (key == "HTTP_PROXY")
            .then(|| "http://http-only:3128".to_string())));
        assert!(has_proxy_environment("https", |key| (key == "ALL_PROXY")
            .then(|| "http://all-proxy:3128".to_string())));
        assert!(!has_proxy_environment("https", |_| Some(String::new())));
    }

    #[tokio::test]
    #[ignore = "requires live Xueqiu access; sends no saved credentials"]
    async fn integration_xueqiu_system_pac_without_proxy_environment() {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            assert!(
                std::env::var(key).unwrap_or_default().is_empty(),
                "remove {key} before running this test"
            );
        }
        let response = xueqiu_get(
            "https://stock.xueqiu.com/v5/stock/realtime/quotec.json?symbol=SH600519,00700,AAPL",
        )
        .await
        .unwrap()
        .send()
        .await
        .unwrap();
        assert!(response.status().is_success());
        assert_eq!(response.url().host_str(), Some("stock.xueqiu.com"));
        let body: serde_json::Value = response.json().await.unwrap();
        let quotes = body["data"].as_array().expect("expected quote JSON");
        assert_eq!(quotes.len(), 3);
        println!("Xueqiu returned {} quotes using system proxy settings without proxy environment variables or saved cookies", quotes.len());
    }

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

    #[test]
    fn test_xueqiu_client_returns_same_instance() {
        let c1 = xueqiu_client();
        let c2 = xueqiu_client();
        assert!(std::ptr::eq(c1, c2));
    }

    #[test]
    fn test_xueqiu_client_can_build_request() {
        let client = xueqiu_client();
        let req = client
            .get("https://stock.xueqiu.com/v5/stock/quote.json?symbol=SH600519")
            .build()
            .expect("should build request");
        assert_eq!(req.method(), reqwest::Method::GET);
    }
}
