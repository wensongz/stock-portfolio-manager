//! Native macOS proxy auto-configuration for Xueqiu requests.

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFIndex, CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::error::{CFError, CFErrorRef};
use core_foundation::number::CFNumber;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRunResult, CFRunLoopSource,
    CFRunLoopSourceInvalidate, CFRunLoopSourceRef,
};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::{CFURLCreateWithString, CFURLRef, CFURL};
use reqwest::Url;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

type ProxyDictionary = CFDictionary<CFString, CFType>;
type ProxyList = CFArray<ProxyDictionary>;
type PacResult = Option<Result<ProxyList, String>>;

const PAC_TIMEOUT: Duration = Duration::from_secs(3);
const PAC_CACHE_TTL: Duration = Duration::from_secs(60);
static PAC_DOWNLOAD_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
static PAC_SCRIPTS: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();

// Nullable callbacks must be Option<fn>, unlike the non-nullable fields in
// core-foundation-sys's CFStreamClientContext binding.
#[repr(C)]
struct StreamContext {
    version: CFIndex,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void) -> CFStringRef>,
}

#[link(name = "CFNetwork", kind = "framework")]
extern "C" {
    fn CFNetworkCopySystemProxySettings() -> CFDictionaryRef;
    fn CFNetworkCopyProxiesForURL(url: CFURLRef, settings: CFDictionaryRef) -> CFArrayRef;
    fn CFNetworkExecuteProxyAutoConfigurationScript(
        script: CFStringRef,
        target_url: CFURLRef,
        callback: extern "C" fn(*mut c_void, CFArrayRef, CFErrorRef),
        context: *mut StreamContext,
    ) -> CFRunLoopSourceRef;
    static kCFProxyTypeKey: CFStringRef;
    static kCFProxyTypeNone: CFStringRef;
    static kCFProxyTypeHTTP: CFStringRef;
    static kCFProxyTypeHTTPS: CFStringRef;
    static kCFProxyTypeSOCKS: CFStringRef;
    static kCFProxyTypeAutoConfigurationURL: CFStringRef;
    static kCFProxyAutoConfigurationURLKey: CFStringRef;
    static kCFProxyHostNameKey: CFStringRef;
    static kCFProxyPortNumberKey: CFStringRef;
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum ProxyRoute {
    Direct,
    Proxy(String),
}

/// Called on a blocking worker: native PAC evaluation downloads JavaScript
/// and runs a CFRunLoop, neither of which belongs on a Tokio executor thread.
pub(super) fn resolve(target: &Url) -> Result<Vec<ProxyRoute>, String> {
    let target = cf_url(target.as_str())?;
    // SAFETY: Copy functions return owned references; null is checked first.
    let settings = unsafe { CFNetworkCopySystemProxySettings() };
    if settings.is_null() {
        return Ok(vec![ProxyRoute::Direct]);
    }
    let settings: ProxyDictionary = unsafe { TCFType::wrap_under_create_rule(settings) };
    let proxies = unsafe {
        CFNetworkCopyProxiesForURL(target.as_concrete_TypeRef(), settings.as_concrete_TypeRef())
    };
    if proxies.is_null() {
        return Err("macOS returned no proxy configuration".to_string());
    }
    let proxies = unsafe { ProxyList::wrap_under_create_rule(proxies) };
    routes_from_list(&proxies, &target)
}

fn cf_url(url: &str) -> Result<CFURL, String> {
    let value = CFString::new(url);
    let url = unsafe {
        CFURLCreateWithString(
            std::ptr::null(),
            value.as_concrete_TypeRef(),
            std::ptr::null(),
        )
    };
    if url.is_null() {
        return Err("Invalid URL for macOS proxy resolution".to_string());
    }
    Ok(unsafe { CFURL::wrap_under_create_rule(url) })
}

fn value(dictionary: &ProxyDictionary, key: CFStringRef) -> Option<CFType> {
    // SAFETY: All keys come from CFNetwork's static CFString constants.
    let key = unsafe { CFString::wrap_under_get_rule(key) };
    dictionary.find(&key).map(|value| value.clone())
}

fn routes_from_list(proxies: &ProxyList, target: &CFURL) -> Result<Vec<ProxyRoute>, String> {
    // CFNetwork orders the list by preference. In particular, DIRECT must
    // not be skipped in favour of a later fallback proxy.
    let mut routes = Vec::new();
    for proxy in proxies.iter() {
        let kind = value(&proxy, unsafe { kCFProxyTypeKey })
            .ok_or_else(|| "macOS proxy has no type".to_string())?;
        let is_type = |expected| {
            let expected = unsafe { CFString::wrap_under_get_rule(expected) };
            kind == expected.as_CFType()
        };
        if is_type(unsafe { kCFProxyTypeNone }) {
            routes.push(ProxyRoute::Direct);
            continue;
        }
        if is_type(unsafe { kCFProxyTypeAutoConfigurationURL }) {
            let pac_url = value(&proxy, unsafe { kCFProxyAutoConfigurationURLKey })
                .and_then(|value| value.downcast::<CFURL>())
                .ok_or_else(|| "macOS PAC configuration has no script URL".to_string())?;
            let proxies = execute_pac(&pac_url, target, PAC_TIMEOUT)?;
            routes.extend(routes_from_list(&proxies, target)?);
            continue;
        }
        // CFProxyTypeHTTPS means an HTTP CONNECT proxy for HTTPS targets.
        let scheme =
            if is_type(unsafe { kCFProxyTypeHTTP }) || is_type(unsafe { kCFProxyTypeHTTPS }) {
                "http"
            } else if is_type(unsafe { kCFProxyTypeSOCKS }) {
                "socks5h"
            } else {
                continue;
            };
        let host = value(&proxy, unsafe { kCFProxyHostNameKey })
            .and_then(|value| value.downcast::<CFString>())
            .map(|value| value.to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "macOS proxy has no hostname".to_string())?;
        let port = value(&proxy, unsafe { kCFProxyPortNumberKey })
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_i32())
            .filter(|value| (1..=65535).contains(value))
            .ok_or_else(|| "macOS proxy has an invalid port".to_string())?;
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host
        };
        routes.push(ProxyRoute::Proxy(format!("{scheme}://{host}:{port}")));
    }
    if routes.is_empty() {
        Err("macOS returned no supported proxy route".to_string())
    } else {
        Ok(routes)
    }
}

#[cfg(test)]
fn resolve_pac(pac_url: &str, target_url: &Url) -> Result<Vec<ProxyRoute>, String> {
    let target = cf_url(target_url.as_str())?;
    let proxies = execute_pac(&cf_url(pac_url)?, &target, PAC_TIMEOUT)?;
    routes_from_list(&proxies, &target)
}

extern "C" fn pac_callback(client: *mut c_void, proxies: CFArrayRef, error: CFErrorRef) {
    // SAFETY: execute_pac owns the result and invalidates the source before
    // returning. This callback runs only on that function's current run loop.
    let result = unsafe { &mut *client.cast::<PacResult>() };
    *result = Some(if !error.is_null() {
        let error = unsafe { CFError::wrap_under_get_rule(error) };
        Err(format!(
            "macOS could not load or evaluate the PAC script ({}: {})",
            error.domain(),
            error.code()
        ))
    } else if proxies.is_null() {
        Err("macOS could not load or evaluate the PAC script".to_string())
    } else {
        // The callback's array is borrowed, so retain it before returning.
        Ok(unsafe { ProxyList::wrap_under_get_rule(proxies) })
    });
}

fn execute_pac(pac: &CFURL, target: &CFURL, timeout: Duration) -> Result<ProxyList, String> {
    // CFNetwork's URL loader attempts HTTPS upgrades even for HTTP PAC URLs.
    // Fetch the configured URL directly, without proxy recursion or cookies,
    // then use the native JavaScript engine to evaluate the downloaded script.
    let script = CFString::new(&load_pac_script(&pac.get_string().to_string())?);
    let mut result: PacResult = None;
    let mut context = StreamContext {
        version: 0,
        info: (&mut result as *mut PacResult).cast(),
        retain: None,
        release: None,
        copy_description: None,
    };
    let source = unsafe {
        CFNetworkExecuteProxyAutoConfigurationScript(
            script.as_concrete_TypeRef(),
            target.as_concrete_TypeRef(),
            pac_callback,
            &mut context,
        )
    };
    if source.is_null() {
        return Err("macOS could not start PAC resolution".to_string());
    }
    let source = unsafe { CFRunLoopSource::wrap_under_create_rule(source) };
    let run_loop = CFRunLoop::get_current();
    let mode = unsafe { kCFRunLoopDefaultMode };
    run_loop.add_source(&source, mode);
    let started = Instant::now();
    while result.is_none() && started.elapsed() < timeout {
        if CFRunLoop::run_in_mode(mode, timeout.saturating_sub(started.elapsed()), true)
            == CFRunLoopRunResult::Finished
        {
            break;
        }
    }
    // Invalidation cancels unfinished work and prevents callbacks from using
    // the stack-owned result after it is dropped, including on timeout.
    unsafe { CFRunLoopSourceInvalidate(source.as_concrete_TypeRef()) };
    run_loop.remove_source(&source, mode);
    result.unwrap_or_else(|| Err("macOS PAC resolution timed out".to_string()))
}

fn load_pac_script(url: &str) -> Result<String, String> {
    let scripts = PAC_SCRIPTS.get_or_init(Default::default);
    if let Some((_, script)) = scripts
        .lock()
        .unwrap()
        .get(url)
        .filter(|(fetched, _)| fetched.elapsed() < PAC_CACHE_TTL)
    {
        return Ok(script.clone());
    }
    let parsed = Url::parse(url).map_err(|_| "Invalid system PAC URL".to_string())?;
    let script = if parsed.scheme() == "file" {
        let path = parsed
            .to_file_path()
            .map_err(|_| "Invalid system PAC file URL".to_string())?;
        std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read system PAC file: {error}"))?
    } else {
        let client = PAC_DOWNLOAD_CLIENT.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(PAC_TIMEOUT)
                .build()
                .expect("failed to build PAC download client")
        });
        client
            .get(parsed)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .and_then(reqwest::blocking::Response::text)
            .map_err(|error| {
                format!(
                    "Failed to download system PAC script: {}",
                    error.without_url()
                )
            })?
    };
    let mut scripts = scripts.lock().unwrap();
    scripts.retain(|_, (fetched, _)| fetched.elapsed() < PAC_CACHE_TTL);
    scripts.insert(url.to_string(), (Instant::now(), script.clone()));
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // Exercise downloading and script execution through a real local server.
    fn serve_once(body: &str) -> (String, std::thread::JoinHandle<String>) {
        serve_response_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ))
    }

    fn serve_response_once(response: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let started = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(5) =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("local proxy server did not receive a request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, handle)
    }

    fn evaluate(script: &str, target: &str) -> Result<Vec<ProxyRoute>, String> {
        let (pac_url, server) = serve_once(script);
        let result = resolve_pac(&pac_url, &Url::parse(target).unwrap());
        server.join().unwrap();
        result
    }

    #[test]
    fn pac_selects_proxy_for_https_xueqiu_and_direct_for_other_origins() {
        let script = r#"function FindProxyForURL(url, host) {
            if (dnsDomainIs(host, ".xueqiu.com") && shExpMatch(url, "https:*"))
                return "PROXY 127.0.0.1:3128";
            return "DIRECT";
        }"#;
        assert_eq!(
            evaluate(
                script,
                "https://stock.xueqiu.com/v5/stock/realtime/quotec.json"
            )
            .unwrap(),
            vec![ProxyRoute::Proxy("http://127.0.0.1:3128".to_string())]
        );
        assert_eq!(
            evaluate(script, "http://stock.xueqiu.com/public").unwrap(),
            vec![ProxyRoute::Direct]
        );
        assert_eq!(
            evaluate(script, "https://example.com/").unwrap(),
            vec![ProxyRoute::Direct]
        );
    }

    #[test]
    fn pac_preserves_direct_before_a_fallback_proxy() {
        assert_eq!(
            evaluate(
                "function FindProxyForURL(url, host) { return 'DIRECT; PROXY 127.0.0.1:3128'; }",
                "https://xueqiu.com/"
            )
            .unwrap(),
            vec![
                ProxyRoute::Direct,
                ProxyRoute::Proxy("http://127.0.0.1:3128".to_string())
            ]
        );
    }

    #[test]
    fn invalid_pac_is_an_error_instead_of_silent_direct_access() {
        assert!(evaluate("function broken {", "https://xueqiu.com/").is_err());
    }

    #[test]
    fn pac_timeout_cancels_execution_and_allows_the_next_resolution() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"function FindProxyForURL(url, host) { while (true) {} }")
            .unwrap();
        file.flush().unwrap();
        let pac = cf_url(Url::from_file_path(file.path()).unwrap().as_str()).unwrap();
        let target = cf_url("https://xueqiu.com/").unwrap();
        let started = Instant::now();
        let error = execute_pac(&pac, &target, Duration::from_millis(50))
            .expect_err("hanging PAC must time out");
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(
            evaluate(
                "function FindProxyForURL(url, host) { return 'DIRECT'; }",
                "https://xueqiu.com/"
            )
            .unwrap(),
            vec![ProxyRoute::Direct]
        );
    }

    #[tokio::test]
    async fn pac_selected_proxy_receives_the_quote_request() {
        let (proxy, server) = serve_once(r#"{"data":[{"symbol":"AAPL","current":100.0}]}"#);
        let script = format!(
            "function FindProxyForURL(url, host) {{ return 'PROXY {}'; }}",
            proxy.trim_start_matches("http://")
        );
        let route = tokio::task::spawn_blocking(move || {
            evaluate(
                &script,
                "http://stock.xueqiu.com/v5/stock/realtime/quotec.json",
            )
        })
        .await
        .unwrap()
        .unwrap();
        let response = super::super::xueqiu_client_for_route(route.into_iter().next().unwrap())
            .unwrap()
            .get("http://stock.xueqiu.com/v5/stock/realtime/quotec.json")
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["data"][0]["symbol"], "AAPL");
        assert!(server
            .join()
            .unwrap()
            .starts_with("GET http://stock.xueqiu.com/v5/stock/realtime/quotec.json HTTP/1.1\r\n"));
    }

    #[tokio::test]
    async fn no_proxy_bypasses_the_selected_system_proxy() {
        let (target, server) = serve_once("direct response");
        let client = super::super::xueqiu_route_builder(
            &ProxyRoute::Proxy("http://127.0.0.1:9".to_string()),
            reqwest::NoProxy::from_string("127.0.0.1"),
        )
        .unwrap()
        .build()
        .unwrap();
        let response = client.get(&target).send().await.unwrap();
        assert_eq!(response.text().await.unwrap(), "direct response");
        assert!(server.join().unwrap().starts_with("GET / HTTP/1.1\r\n"));
    }

    #[tokio::test]
    async fn request_tries_the_next_pac_proxy_when_the_first_is_unavailable() {
        let (proxy, server) = serve_once("fallback response");
        let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
        let unavailable_url = format!("http://{}", unavailable.local_addr().unwrap());
        drop(unavailable);
        let requests = [unavailable_url, proxy]
            .into_iter()
            .map(|proxy| {
                super::super::xueqiu_route_builder(&ProxyRoute::Proxy(proxy), None)
                    .unwrap()
                    .build()
                    .unwrap()
                    .get("http://stock.xueqiu.com/v5/stock/realtime/quotec.json")
            })
            .collect();
        let response = super::super::XueqiuRequest { requests }
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "fallback response");
        assert!(server
            .join()
            .unwrap()
            .starts_with("GET http://stock.xueqiu.com/"));
    }

    #[tokio::test]
    async fn verification_redirect_is_identified_without_contacting_its_destination() {
        let (url, server) = serve_response_once("HTTP/1.1 302 Found\r\nLocation: http://check.xueqiu.com:9/captcha\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string());
        let client = super::super::xueqiu_client_builder()
            .no_proxy()
            .resolve("check.xueqiu.com", "127.0.0.1:9".parse().unwrap())
            .build()
            .unwrap();
        let response = client.get(url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        assert_eq!(
            response.headers()[reqwest::header::LOCATION],
            "http://check.xueqiu.com:9/captcha"
        );
        server.join().unwrap();
    }
}
