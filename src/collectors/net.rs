//! Shared network clients and a careful HTTP fetch used by every network collector.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use hickory_resolver::TokioResolver;
use reqwest::header::{ACCEPT, HeaderMap, LOCATION};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::config::{NetworkConfig, duration_from_secs};
use crate::model::target::is_non_public_ip;
use crate::sanitize::sanitize_line;

const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const DNS_ATTEMPTS: usize = 2;

#[derive(Clone, Debug)]
pub struct NetSettings {
    pub user_agent: String,
    pub request_timeout: Duration,
    pub max_body_bytes: usize,
    pub max_redirects: usize,
    pub allow_private_redirects: bool,
    pub proxy: Option<String>,
}

/// HTTP client and DNS resolver shared by collectors.
pub struct NetContext {
    pub http: reqwest::Client,
    pub dns: TokioResolver,
    pub settings: NetSettings,
}

impl std::fmt::Debug for NetContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetContext")
            .field("settings", &self.settings)
            .finish()
    }
}

impl NetContext {
    pub fn new(config: &NetworkConfig) -> Result<Self, String> {
        let request_timeout = duration_from_secs(config.request_timeout_seconds.max(1));
        let mut builder = reqwest::Client::builder()
            .user_agent(config.user_agent.clone())
            .timeout(request_timeout)
            .connect_timeout(duration_from_secs(config.connect_timeout_seconds.max(1)))
            .redirect(reqwest::redirect::Policy::none());
        let proxy = config
            .proxy
            .clone()
            .filter(|value| !value.trim().is_empty());
        if let Some(proxy_url) = &proxy {
            let proxy = reqwest::Proxy::all(proxy_url)
                .map_err(|error| format!("invalid network.proxy: {error}"))?;
            builder = builder.proxy(proxy);
        }
        let http = builder
            .build()
            .map_err(|error| format!("could not create HTTP client: {error}"))?;
        let mut resolver_builder = hickory_resolver::Resolver::builder_tokio()
            .map_err(|error| format!("could not read the system DNS configuration: {error}"))?;
        resolver_builder.options_mut().timeout = DNS_TIMEOUT;
        resolver_builder.options_mut().attempts = DNS_ATTEMPTS;
        let dns = resolver_builder
            .build()
            .map_err(|error| format!("could not create DNS resolver: {error}"))?;
        Ok(Self {
            http,
            dns,
            settings: NetSettings {
                user_agent: config.user_agent.clone(),
                request_timeout,
                max_body_bytes: config.max_body_bytes,
                max_redirects: config.max_redirects,
                allow_private_redirects: config.allow_private_redirects,
                proxy,
            },
        })
    }
}

/// One hop of a redirect chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedirectHop {
    pub url: Url,
    pub status: u16,
    pub location: String,
}

#[derive(Clone, Debug)]
pub struct FetchedResponse {
    pub requested_url: Url,
    pub final_url: Url,
    pub redirects: Vec<RedirectHop>,
    pub status: u16,
    /// Header names lowercased; values sanitized.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_truncated: bool,
    pub remote_addr: Option<SocketAddr>,
    pub elapsed: Duration,
    /// Set when a redirect was refused (for example to a private address).
    pub blocked_redirect: Option<String>,
}

impl FetchedResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    pub fn is_html(&self) -> bool {
        self.content_type()
            .map(|value| value.contains("html"))
            .unwrap_or_else(|| {
                self.body.starts_with(b"<")
                    || self
                        .body
                        .windows(5)
                        .take(512)
                        .any(|window| window.eq_ignore_ascii_case(b"<html"))
            })
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Options for a single fetch.
#[derive(Clone, Debug)]
pub struct FetchOptions {
    pub max_body_bytes: usize,
    pub accept: Option<&'static str>,
    /// Extra request headers (API keys). Callers redact logs separately.
    pub extra_headers: Vec<(String, String)>,
}

/// Fetches a URL, following redirects manually so the chain is recorded and redirects from public
/// hosts to private or loopback addresses can be refused. The body is capped at
/// `max_body_bytes`.
pub async fn fetch(
    net: &NetContext,
    url: &Url,
    options: &FetchOptions,
    cancel: &CancellationToken,
) -> Result<FetchedResponse, String> {
    let started = Instant::now();
    let starts_private = host_is_non_public(url);
    let mut current = url.clone();
    let mut redirects = Vec::new();
    loop {
        let response = send(net, &current, options, cancel).await?;
        let status = response.status().as_u16();
        let location = location_of(response.headers());
        let is_redirect = (300..400).contains(&status) && location.is_some();
        if !is_redirect {
            return read_response(
                response,
                url.clone(),
                current,
                redirects,
                options,
                cancel,
                started,
                None,
            )
            .await;
        }
        let location = location.unwrap_or_default();
        let next = current
            .join(&location)
            .map_err(|error| format!("invalid redirect location {location:?}: {error}"))?;
        redirects.push(RedirectHop {
            url: current.clone(),
            status,
            location: sanitize_line(&location).into_owned(),
        });
        if let Some(reason) = refuse_redirect(net, &next, starts_private, redirects.len()) {
            return read_response(
                response,
                url.clone(),
                current,
                redirects,
                options,
                cancel,
                started,
                Some(reason),
            )
            .await;
        }
        current = next;
    }
}

fn refuse_redirect(
    net: &NetContext,
    next: &Url,
    starts_private: bool,
    hops: usize,
) -> Option<String> {
    if !matches!(next.scheme(), "http" | "https") {
        return Some(format!(
            "refused redirect to unsupported scheme {}://",
            next.scheme()
        ));
    }
    if hops > net.settings.max_redirects {
        return Some(format!(
            "stopped after {} redirects (network.max_redirects)",
            net.settings.max_redirects
        ));
    }
    if !starts_private && !net.settings.allow_private_redirects && host_is_non_public(next) {
        return Some(format!(
            "refused redirect from a public host to non-public address {} (network.allow_private_redirects)",
            next.host_str().unwrap_or_default()
        ));
    }
    None
}

async fn send(
    net: &NetContext,
    url: &Url,
    options: &FetchOptions,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, String> {
    let mut request = net.http.get(url.clone());
    if let Some(accept) = options.accept {
        request = request.header(ACCEPT, accept);
    }
    for (name, value) in &options.extra_headers {
        request = request.header(name.as_str(), value.as_str());
    }
    tokio::select! {
        _ = cancel.cancelled() => Err("cancelled".to_string()),
        response = request.send() => response.map_err(|error| describe_reqwest_error(url, &error)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_response(
    mut response: reqwest::Response,
    requested_url: Url,
    final_url: Url,
    redirects: Vec<RedirectHop>,
    options: &FetchOptions,
    cancel: &CancellationToken,
    started: Instant,
    blocked_redirect: Option<String>,
) -> Result<FetchedResponse, String> {
    let status = response.status().as_u16();
    let headers = sanitized_headers(response.headers());
    let remote_addr = response.remote_addr();
    let mut body = Vec::new();
    let mut body_truncated = false;
    if blocked_redirect.is_none() {
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err("cancelled".to_string()),
                chunk = response.chunk() => chunk.map_err(|error| describe_reqwest_error(&final_url, &error))?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let remaining = options.max_body_bytes.saturating_sub(body.len());
            if chunk.len() > remaining {
                body.extend_from_slice(&chunk[..remaining]);
                body_truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
    }
    Ok(FetchedResponse {
        requested_url,
        final_url,
        redirects,
        status,
        headers,
        body,
        body_truncated,
        remote_addr,
        elapsed: started.elapsed(),
        blocked_redirect,
    })
}

fn location_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(LOCATION)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn sanitized_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let value = String::from_utf8_lossy(value.as_bytes());
            (
                name.as_str().to_ascii_lowercase(),
                sanitize_line(&value).into_owned(),
            )
        })
        .collect()
}

fn host_is_non_public(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => is_non_public_ip(&IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_non_public_ip(&IpAddr::V6(ip)),
        Some(url::Host::Domain(domain)) => {
            let lower = domain.to_ascii_lowercase();
            lower == "localhost"
                || lower.ends_with(".localhost")
                || lower.ends_with(".local")
                || lower.ends_with(".internal")
        }
        None => true,
    }
}

/// Turns reqwest errors into short, actionable messages.
pub fn describe_reqwest_error(url: &Url, error: &reqwest::Error) -> String {
    let host = url.host_str().unwrap_or_default();
    if error.is_timeout() {
        return format!("request to {host} timed out (network.request_timeout_seconds)");
    }
    if error.is_connect() {
        return format!("could not connect to {host}: {}", root_cause(error));
    }
    format!("request to {host} failed: {}", root_cause(error))
}

fn root_cause(error: &(dyn std::error::Error + 'static)) -> String {
    let mut current = error;
    while let Some(source) = current.source() {
        current = source;
    }
    current.to_string()
}

/// Maps HTTP status codes from API-style services to user-facing errors.
pub fn describe_status(service: &str, status: u16) -> String {
    let meaning = match status {
        400 => "bad request",
        401 | 403 => "access denied",
        404 => "not found",
        429 => "rate limited",
        500..=599 => "server error",
        _ => "unexpected response",
    };
    format!("{service} failed: HTTP {status}: {meaning}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_public_hosts_are_detected() {
        let private = Url::parse("http://192.168.1.1/").expect("url");
        let loopback = Url::parse("http://[::1]:8080/").expect("url");
        let localhost = Url::parse("http://localhost/").expect("url");
        let public = Url::parse("https://example.com/").expect("url");
        assert!(host_is_non_public(&private));
        assert!(host_is_non_public(&loopback));
        assert!(host_is_non_public(&localhost));
        assert!(!host_is_non_public(&public));
    }

    #[test]
    fn status_descriptions_are_readable() {
        assert_eq!(
            describe_status("RDAP collection", 429),
            "RDAP collection failed: HTTP 429: rate limited"
        );
    }
}
