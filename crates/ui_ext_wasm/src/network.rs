//! `network:fetch` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §8, plan 4.6, spec decision 23): a
//! host import, not WASI sockets (the WASI context denies those, [`crate::deny_network`]).
//!
//! - [`DomainList`]: the manifest's `[fyx_ui.network] domains`, parsed at load.
//! - [`check_request`]: what the import checks at once. An undeclared host is `denied`.
//! - [`Fetcher`]: runs a checked request after the call returns (design §0.3). The host follows
//!   redirects itself, checking every hop like the first. It resolves each name once, refuses
//!   any non-global address for a public domain, and connects only to the addresses it checked
//!   (DNS rebinding). It caps the decoded body while streaming.
//!
//! The import, the counters and the delivery through `handle-io` are in `fyx_ui_v0_3.rs`,
//! `effects.rs` and [`crate::UiExtension::handle_io`].

use std::{
    collections::HashMap,
    fmt, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use futures::{
    AsyncReadExt as _, FutureExt as _, SinkExt as _, TryStreamExt as _, future::BoxFuture,
};
use http_proxy::HostPattern;
use ui_ext::v0_2::{HostError, HostErrorCode};
use url::Url;

pub const CAPABILITY_NETWORK: &str = "network:fetch";
/// Requests in flight per instance, counted until their `fetch-done` is delivered (decision 23).
pub const MAX_IN_FLIGHT: usize = 8;
/// Requests started per rolling minute.
pub const MAX_STARTED_PER_MINUTE: usize = 120;
pub const MAX_URL_BYTES: usize = 8 * 1024;
pub const MAX_REQUEST_HEADERS: usize = 64;
pub const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;
pub const MAX_REQUEST_BODY_BYTES: usize = 4 << 20;
/// The decoded response body; a longer one ends with [`FetchError::TooLarge`].
pub const MAX_RESPONSE_BODY_BYTES: u64 = 16 << 20;
pub const MAX_RESPONSE_HEADER_BYTES: usize = 64 * 1024;
/// Redirects followed; the next one ends with [`FetchError::RedirectNotAllowed`].
pub const MAX_REDIRECTS: usize = 5;
pub const MAX_TIMEOUT: Duration = Duration::from_secs(30);
/// Domain patterns per manifest.
pub const MAX_DOMAINS: usize = 64;

/// Request headers the host sets or that frame the request, refused at the import (WIT
/// `http-request.headers`), plus `accept-encoding`: the host decodes the body, so it chooses.
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "keep-alive",
    "accept-encoding",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
}

impl Method {
    fn as_reqwest(self) -> reqwest::Method {
        match self {
            Method::Get => reqwest::Method::GET,
            Method::Head => reqwest::Method::HEAD,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
            Method::Delete => reqwest::Method::DELETE,
            Method::Options => reqwest::Method::OPTIONS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The URL that answered, after the redirects the host followed.
    pub url: String,
}

/// The WIT `fetch-error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Denied,
    HostNotAllowed(String),
    RedirectNotAllowed(String),
    Dns(String),
    Connect(String),
    Tls(String),
    Timeout,
    TooLarge,
    Cancelled,
    Other(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Denied => f.write_str("denied"),
            FetchError::HostNotAllowed(host) => write!(f, "host not allowed: {host}"),
            FetchError::RedirectNotAllowed(host) => write!(f, "redirect not allowed: {host}"),
            FetchError::Dns(error) => write!(f, "dns: {error}"),
            FetchError::Connect(error) => write!(f, "connect: {error}"),
            FetchError::Tls(error) => write!(f, "tls: {error}"),
            FetchError::Timeout => f.write_str("timeout"),
            FetchError::TooLarge => f.write_str("too large"),
            FetchError::Cancelled => f.write_str("cancelled"),
            FetchError::Other(error) => f.write_str(error),
        }
    }
}

/// What the host delivers through the `handle-io` export (design §0.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IoEvent {
    FetchDone(u64, Result<FetchResponse, FetchError>),
}

// ---------------------------------------------------------------------------------------------
// The domain list

#[derive(Clone, Debug, PartialEq, Eq)]
enum PatternHost {
    Name(HostPattern),
    /// `localhost:<port>` or `127.0.0.1:<port>`: `http://` allowed, never resolved through DNS.
    Loopback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DomainPattern {
    text: String,
    host: PatternHost,
    /// 443 unless the pattern names one; a loopback pattern always names one.
    port: Option<u16>,
}

/// A host the list allows, with what that allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Allowed {
    loopback: bool,
}

/// The manifest's `[fyx_ui.network] domains` (design §8, "Domain list").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainList {
    patterns: Vec<DomainPattern>,
}

impl DomainList {
    /// Parses the patterns, refusing what the loader refuses: a wildcard whose base is a public
    /// suffix (`*.com`, `*.github.io`), an IP literal other than `127.0.0.1`, `localhost` or
    /// `127.0.0.1` without a port, more than [`MAX_DOMAINS`].
    pub fn parse(patterns: &[String]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            patterns.len() <= MAX_DOMAINS,
            "at most {MAX_DOMAINS} network domains, got {}",
            patterns.len()
        );
        let mut parsed = Vec::new();
        for text in patterns {
            parsed.push(parse_pattern(text)?);
        }
        Ok(Self { patterns: parsed })
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// The patterns as written, for the consent prompt.
    pub fn texts(&self) -> Vec<String> {
        self.patterns.iter().map(|p| p.text.clone()).collect()
    }

    fn allows(&self, url: &Url) -> Option<Allowed> {
        let port = url.port_or_known_default()?;
        let host = url.host()?;
        for pattern in &self.patterns {
            let matches = match (&pattern.host, &host) {
                (PatternHost::Loopback, url::Host::Domain(name)) => {
                    name.eq_ignore_ascii_case("localhost")
                }
                (PatternHost::Loopback, url::Host::Ipv4(ip)) => ip.is_loopback(),
                (PatternHost::Loopback, url::Host::Ipv6(_)) => false,
                // IP literals never match a domain pattern.
                (PatternHost::Name(name), url::Host::Domain(domain)) => name.matches(domain),
                (PatternHost::Name(_), _) => false,
            };
            if matches && pattern.port.unwrap_or(443) == port {
                return Some(Allowed {
                    loopback: pattern.host == PatternHost::Loopback,
                });
            }
        }
        None
    }
}

fn parse_pattern(text: &str) -> anyhow::Result<DomainPattern> {
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            let port: u16 = port
                .parse()
                .map_err(|_| anyhow::anyhow!("network domain {text:?}: {port:?} is not a port"))?;
            anyhow::ensure!(port != 0, "network domain {text:?}: port 0");
            (host, Some(port))
        }
        _ => (text, None),
    };
    if host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" {
        anyhow::ensure!(
            port.is_some(),
            "network domain {text:?}: localhost needs a port, such as localhost:8080"
        );
        return Ok(DomainPattern {
            text: text.to_string(),
            host: PatternHost::Loopback,
            port,
        });
    }
    let pattern = HostPattern::parse(host)
        .map_err(|error| anyhow::anyhow!("network domain {text:?}: {error}"))?;
    // A wildcard over a public suffix (`*.com`, `*.github.io`) would allow every site under it;
    // an exact host that is one is a site of its own (`github.io`), unless it is a bare label.
    match &pattern {
        HostPattern::Subdomain(base) => anyhow::ensure!(
            !fyx_registry::is_public_suffix(base),
            "network domain {text:?}: {base} is a public suffix, so the pattern would allow \
             every site under it"
        ),
        HostPattern::Exact(host) => anyhow::ensure!(
            host.contains('.'),
            "network domain {text:?}: {host} is a top-level domain, not a host"
        ),
    }
    Ok(DomainPattern {
        text: text.to_string(),
        host: PatternHost::Name(pattern),
        port,
    })
}

// ---------------------------------------------------------------------------------------------
// The checks at the import

fn invalid(message: impl Into<String>) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message: message.into(),
    }
}

/// Everything the import can check at once (design §8, "Semantics"). Syntax errors are
/// `invalid-argument`; a host the manifest does not list, or `http://` to one that is not a
/// declared loopback, is `denied` (plan 4.6).
pub fn check_request(request: &FetchRequest, domains: &DomainList) -> Result<Url, HostError> {
    if request.url.len() > MAX_URL_BYTES {
        return Err(invalid(format!("the URL is over {MAX_URL_BYTES} bytes")));
    }
    let url = Url::parse(&request.url).map_err(|error| invalid(format!("bad URL: {error}")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("the URL may not carry user info"));
    }
    if !matches!(url.scheme(), "https" | "http") {
        return Err(invalid(format!("scheme {} is not allowed", url.scheme())));
    }
    let Some(allowed) = domains.allows(&url) else {
        return Err(HostError {
            code: HostErrorCode::Denied,
            message: format!(
                "{} is not in the extension's network domains",
                host_and_port(&url)
            ),
        });
    };
    if url.scheme() == "http" && !allowed.loopback {
        return Err(HostError {
            code: HostErrorCode::Denied,
            message: "http:// is allowed only to a declared localhost port".into(),
        });
    }
    if request.headers.len() > MAX_REQUEST_HEADERS {
        return Err(invalid(format!("at most {MAX_REQUEST_HEADERS} headers")));
    }
    let mut header_bytes = 0;
    for (name, value) in &request.headers {
        header_bytes += name.len() + value.len();
        let lower = name.to_ascii_lowercase();
        if FORBIDDEN_HEADERS.contains(&lower.as_str()) || lower.starts_with("proxy-") {
            return Err(invalid(format!("header {name} is set by the host")));
        }
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| invalid(format!("bad header name {name:?}")))?;
        reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| invalid(format!("bad value for header {name}")))?;
    }
    if header_bytes > MAX_REQUEST_HEADER_BYTES {
        return Err(invalid(format!(
            "the headers are over {MAX_REQUEST_HEADER_BYTES} bytes"
        )));
    }
    if request.body.as_ref().map_or(0, Vec::len) > MAX_REQUEST_BODY_BYTES {
        return Err(invalid(format!(
            "the body is over {MAX_REQUEST_BODY_BYTES} bytes"
        )));
    }
    if request
        .timeout_ms
        .is_some_and(|ms| ms == 0 || u128::from(ms) > MAX_TIMEOUT.as_millis())
    {
        return Err(invalid("timeout-ms must be 1 to 30000"));
    }
    Ok(url)
}

fn host_and_port(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        _ => url.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// Addresses

/// Whether a public domain may resolve to `ip`: only global unicast addresses (design §8,
/// "Addresses"): not loopback, private, link-local (cloud metadata included), CGNAT,
/// unspecified, broadcast, multicast, documentation, benchmarking or reserved ranges, nor an
/// IPv4-mapped or -compatible form of these.
pub fn is_global(ip: IpAddr) -> bool {
    if http_proxy::is_forbidden_ip(ip) {
        return false;
    }
    match ip {
        IpAddr::V4(ip) => is_global_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped().or_else(|| ip.to_ipv4()) {
                return is_global_v4(v4);
            }
            let first = ip.segments()[0];
            !(ip.is_multicast()
                // 2001:db8::/32 documentation
                || (first == 0x2001 && ip.segments()[1] == 0x0db8)
                // 64:ff9b::/96 and 64:ff9b:1::/48 translation could reach anything
                || first == 0x0064
                // 100::/64 discard-only
                || (first == 0x0100 && ip.segments()[1..4] == [0, 0, 0]))
        }
    }
}

fn is_global_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(http_proxy::is_forbidden_ip(IpAddr::V4(ip))
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || a >= 240
        // 198.18.0.0/15 benchmarking
        || (a == 198 && (b & 0xfe) == 18)
        // 192.0.0.0/24 protocol assignments
        || (a == 192 && b == 0 && c == 0))
}

/// Name resolution, replaceable in tests.
pub trait Resolve: Send + Sync + 'static {
    fn resolve(&self, host: &str, port: u16) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>>;
}

/// The system resolver, through tokio.
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> {
        let host = host.to_string();
        async move {
            Ok(tokio::net::lookup_host((host.as_str(), port))
                .await?
                .collect())
        }
        .boxed()
    }
}

// ---------------------------------------------------------------------------------------------
// The engine

/// Runs checked requests. One per extension instance.
#[derive(Clone)]
pub struct Fetcher {
    resolver: Arc<dyn Resolve>,
    user_agent: String,
    /// Test only: which addresses count as global, so a test server on loopback can stand in
    /// for a public host.
    is_global: fn(IpAddr) -> bool,
}

impl Fetcher {
    pub fn new(extension_id: &str, resolver: Option<Arc<dyn Resolve>>) -> Self {
        Self {
            resolver: resolver.unwrap_or_else(|| Arc::new(SystemResolver)),
            user_agent: format!("Fyx (extension {extension_id})"),
            is_global,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_address_rule(mut self, is_global: fn(IpAddr) -> bool) -> Self {
        self.is_global = is_global;
        self
    }

    /// Runs `request` (checked by [`check_request`] into `url`) to its response or error,
    /// within its timeout.
    pub async fn fetch(
        &self,
        domains: &DomainList,
        url: Url,
        request: FetchRequest,
    ) -> Result<FetchResponse, FetchError> {
        let timeout = request
            .timeout_ms
            .map(|ms| Duration::from_millis(u64::from(ms)))
            .unwrap_or(MAX_TIMEOUT)
            .min(MAX_TIMEOUT);
        match tokio::time::timeout(timeout, self.follow(domains, url, request)).await {
            Ok(result) => result,
            Err(_) => Err(FetchError::Timeout),
        }
    }

    async fn follow(
        &self,
        domains: &DomainList,
        mut url: Url,
        request: FetchRequest,
    ) -> Result<FetchResponse, FetchError> {
        let mut method = request.method;
        let mut body = request.body;
        for hop in 0..=MAX_REDIRECTS {
            let allowed = match domains.allows(&url) {
                Some(allowed) if url.scheme() == "https" || allowed.loopback => allowed,
                _ if hop == 0 => {
                    return Err(FetchError::HostNotAllowed(host_and_port(&url)));
                }
                _ => return Err(FetchError::RedirectNotAllowed(host_and_port(&url))),
            };
            let response = self
                .send(&url, allowed, method, &request.headers, body.clone())
                .await?;
            let status = response.status();
            if status.is_redirection()
                && let Some(location) = response.headers().get(reqwest::header::LOCATION)
            {
                let next = location
                    .to_str()
                    .ok()
                    .and_then(|location| url.join(location).ok())
                    .ok_or_else(|| FetchError::Other("a redirect with a bad Location".into()))?;
                if hop == MAX_REDIRECTS {
                    return Err(FetchError::RedirectNotAllowed(host_and_port(&next)));
                }
                // As browsers do: 303, and 301/302 after a POST, continue as a GET without body.
                if status == reqwest::StatusCode::SEE_OTHER
                    || (matches!(status.as_u16(), 301 | 302) && method == Method::Post)
                {
                    method = Method::Get;
                    body = None;
                }
                url = next;
                continue;
            }
            return read_response(response, method, url).await;
        }
        unreachable!("the loop returns by the last hop")
    }

    async fn send(
        &self,
        url: &Url,
        allowed: Allowed,
        method: Method,
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response, FetchError> {
        let host = url
            .host_str()
            .ok_or_else(|| FetchError::Other("no host".into()))?
            .to_string();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| FetchError::Other("no port".into()))?;
        let mut builder = reqwest::Client::builder()
            .redirect_policy(reqwest::redirect::Policy::none())
            // No proxy from the environment: those can carry the user's credentials (decision
            // 23). Fyx's own proxy setting is not applied yet.
            .no_proxy()
            .user_agent(self.user_agent.clone());
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            // Only `127.0.0.1` gets here (IP literals never match a domain pattern).
            if !allowed.loopback || !ip.is_loopback() {
                return Err(FetchError::HostNotAllowed(host));
            }
        } else {
            let addrs = if allowed.loopback {
                // `localhost` never goes to DNS.
                vec![
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
                    SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
                ]
            } else {
                let addrs = self
                    .resolver
                    .resolve(&host, port)
                    .await
                    .map_err(|error| FetchError::Dns(format!("{host}: {error}")))?;
                if addrs.is_empty() {
                    return Err(FetchError::Dns(format!("{host}: no address")));
                }
                // Every address must be global; one private answer is enough to refuse, so a
                // name cannot mix a public and a private address to reach the local network.
                if let Some(addr) = addrs.iter().find(|addr| !(self.is_global)(addr.ip())) {
                    return Err(FetchError::HostNotAllowed(format!(
                        "{host} (resolves to {})",
                        addr.ip()
                    )));
                }
                addrs
            };
            // Connect to the checked addresses only: reqwest does not resolve this name again.
            builder = builder.resolve_to_addrs(&host, &addrs);
        }
        let client = builder
            .build()
            .map_err(|error| FetchError::Other(format!("{error}")))?;
        let mut request = client
            .request(method.as_reqwest(), url.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "gzip");
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        request.send().await.map_err(send_error)
    }
}

fn send_error(error: reqwest::Error) -> FetchError {
    if error.is_timeout() {
        return FetchError::Timeout;
    }
    let chain = {
        let mut text = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            text.push_str(": ");
            text.push_str(&cause.to_string());
            source = cause.source();
        }
        text
    };
    let lower = chain.to_ascii_lowercase();
    if lower.contains("certificate") || lower.contains("tls") || lower.contains("handshake") {
        FetchError::Tls(chain)
    } else if error.is_connect() {
        FetchError::Connect(chain)
    } else {
        FetchError::Other(chain)
    }
}

async fn read_response(
    response: reqwest::Response,
    method: Method,
    url: Url,
) -> Result<FetchResponse, FetchError> {
    let status = response.status().as_u16();
    let mut header_bytes = 0;
    let mut headers = Vec::new();
    for (name, value) in response.headers() {
        header_bytes += name.as_str().len() + value.len();
        if header_bytes > MAX_RESPONSE_HEADER_BYTES {
            return Err(FetchError::TooLarge);
        }
        headers.push((
            name.as_str().to_string(),
            String::from_utf8_lossy(value.as_bytes()).into_owned(),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES)
    {
        return Err(FetchError::TooLarge);
    }
    let encoding = response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_ascii_lowercase());
    let body = if method == Method::Head {
        Vec::new()
    } else {
        let raw = Box::pin(
            response
                .bytes_stream()
                .map_err(|error| io::Error::other(error.to_string())),
        )
        .into_async_read();
        match encoding.as_deref() {
            None | Some("") | Some("identity") => read_capped(raw).await?,
            Some("gzip") | Some("x-gzip") => {
                read_capped(async_compression::futures::bufread::GzipDecoder::new(
                    futures::io::BufReader::new(raw),
                ))
                .await?
            }
            Some(other) => {
                return Err(FetchError::Other(format!(
                    "unsupported content-encoding {other}"
                )));
            }
        }
    };
    if encoding.is_some() {
        // The body is decoded, so these no longer describe it.
        headers.retain(|(name, _)| name != "content-encoding" && name != "content-length");
    }
    Ok(FetchResponse {
        status,
        headers,
        body,
        url: url.to_string(),
    })
}

/// Reads to the end, failing with `too-large` as soon as the decoded body passes the cap, so a
/// compression bomb never inflates past it.
async fn read_capped(mut reader: impl futures::AsyncRead + Unpin) -> Result<Vec<u8>, FetchError> {
    // A fixed buffer: `read_to_end` over these readers is quadratic in debug builds.
    let mut body = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| FetchError::Connect(error.to_string()))?;
        if read == 0 {
            return Ok(body);
        }
        if (body.len() + read) as u64 > MAX_RESPONSE_BODY_BYTES {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&buffer[..read]);
    }
}

// ---------------------------------------------------------------------------------------------
// Running requests

/// The requests of one instance that started and have not delivered yet, so `cancel-fetch`,
/// a reload and a disable can stop them.
#[derive(Default)]
pub(crate) struct Running {
    handles: Mutex<HashMap<u64, futures::future::AbortHandle>>,
    /// Set when the instance goes away: its aborted requests deliver nothing.
    closed: std::sync::atomic::AtomicBool,
}

impl Running {
    pub(crate) fn insert(&self, id: u64, handle: futures::future::AbortHandle) {
        self.lock().insert(id, handle);
    }

    pub(crate) fn remove(&self, id: u64) {
        self.lock().remove(&id);
    }

    /// Aborts `id`; its task then reports [`FetchError::Cancelled`]. False if it is not running.
    pub(crate) fn cancel(&self, id: u64) -> bool {
        match self.lock().remove(&id) {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        }
    }

    /// The instance is gone (reload, disable, uninstall): every request stops and none
    /// delivers, not even `cancelled` (design §0.3 step 7).
    pub(crate) fn cancel_all(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        for (_, handle) in self.lock().drain() {
            handle.abort();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, futures::future::AbortHandle>> {
        self.handles.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A request the import accepted, started once the call returns successfully.
pub(crate) struct PendingFetch {
    id: u64,
    url: Url,
    request: FetchRequest,
    /// `cancel-fetch` in the same call: it delivers `cancelled` without starting.
    cancelled: bool,
}

/// The `network` state of one instance, in its store: ids, counters, the requests of the running
/// call. Requests count as in flight from the import until their `fetch-done` is delivered.
pub(crate) struct NetworkState {
    /// Empty unless the manifest declares `network:fetch` with domains.
    domains: Arc<DomainList>,
    next_id: u64,
    in_flight: std::collections::BTreeSet<u64>,
    started: std::collections::VecDeque<std::time::Instant>,
    pending: Vec<PendingFetch>,
    running: Arc<Running>,
}

impl NetworkState {
    pub(crate) fn new(domains: Arc<DomainList>, running: Arc<Running>) -> Self {
        Self {
            domains,
            next_id: 1,
            in_flight: Default::default(),
            started: Default::default(),
            pending: Vec::new(),
            running,
        }
    }

    /// `network.fetch`: the checks of [`check_request`], then the in-flight and rate caps. The
    /// caller has checked the capability with the broker.
    pub(crate) fn fetch(&mut self, request: FetchRequest) -> Result<u64, HostError> {
        if self.domains.is_empty() {
            return Err(HostError {
                code: HostErrorCode::NotPermitted,
                message: format!(
                    "fetch needs {CAPABILITY_NETWORK:?} and [fyx_ui.network] domains in the manifest"
                ),
            });
        }
        let url = check_request(&request, &self.domains)?;
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return Err(HostError {
                code: HostErrorCode::LimitExceeded,
                message: format!("at most {MAX_IN_FLIGHT} requests in flight"),
            });
        }
        let now = std::time::Instant::now();
        while self
            .started
            .front()
            .is_some_and(|started| now.duration_since(*started) >= Duration::from_secs(60))
        {
            self.started.pop_front();
        }
        if self.started.len() >= MAX_STARTED_PER_MINUTE {
            return Err(HostError {
                code: HostErrorCode::LimitExceeded,
                message: format!("at most {MAX_STARTED_PER_MINUTE} requests per minute"),
            });
        }
        let id = self.next_id;
        self.next_id += 1;
        self.in_flight.insert(id);
        self.started.push_back(now);
        self.pending.push(PendingFetch {
            id,
            url,
            request,
            cancelled: false,
        });
        Ok(id)
    }

    /// `network.cancel-fetch`: unknown or finished ids are ignored.
    pub(crate) fn cancel(&mut self, id: u64) {
        if let Some(pending) = self.pending.iter_mut().find(|pending| pending.id == id) {
            pending.cancelled = true;
        } else {
            self.running.cancel(id);
        }
    }

    /// After a call: the requests to start if it succeeded. A failed call (trap, budget) drops
    /// them with the rest of its effects (design §0.3 step 2), and they never count.
    pub(crate) fn finish_call(&mut self, succeeded: bool) -> Vec<PendingFetch> {
        let pending = std::mem::take(&mut self.pending);
        if succeeded {
            pending
        } else {
            for fetch in &pending {
                self.in_flight.remove(&fetch.id);
            }
            Vec::new()
        }
    }

    /// Before `handle-io` delivers `fetch-done`: the request is no longer in flight.
    pub(crate) fn delivered(&mut self, id: u64) {
        self.in_flight.remove(&id);
    }
}

/// Starts accepted requests and hands their outcomes to the host, once per instance. Dropping
/// the last [`crate::UiExtension`] clone cancels every running request, so a reloaded or
/// disabled instance gets no outcome of the old one's ids (design §0.3 step 7).
pub(crate) struct NetworkHandle {
    extension_id: Arc<str>,
    domains: Arc<DomainList>,
    fetcher: Fetcher,
    consent: Option<Arc<dyn crate::permissions::T2Consent>>,
    running: Arc<Running>,
    events: futures::channel::mpsc::Sender<IoEvent>,
    receiver: Mutex<Option<futures::channel::mpsc::Receiver<IoEvent>>>,
}

/// `handle-io` calls waiting per instance (design §0.3 step 6). A full queue holds finished
/// requests back; nothing is dropped.
pub const MAX_QUEUED_IO_EVENTS: usize = 256;

impl NetworkHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        domains: Arc<DomainList>,
        fetcher: Fetcher,
        consent: Option<Arc<dyn crate::permissions::T2Consent>>,
        running: Arc<Running>,
    ) -> Self {
        let (events, receiver) = futures::channel::mpsc::channel(MAX_QUEUED_IO_EVENTS);
        Self {
            extension_id,
            domains,
            fetcher,
            consent,
            running,
            events,
            receiver: Mutex::new(Some(receiver)),
        }
    }

    pub(crate) fn take_events(&self) -> Option<futures::channel::mpsc::Receiver<IoEvent>> {
        self.receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Whether the capability is granted now, for the delivery: a revocation while a request
    /// ran turns its outcome into `denied`.
    pub(crate) fn granted(&self) -> bool {
        self.consent.as_ref().is_some_and(|consent| {
            consent.state(&self.extension_id, CAPABILITY_NETWORK)
                == crate::permissions::ConsentState::Granted
        })
    }

    /// Starts `pending` on the current tokio runtime, in call order.
    pub(crate) fn start(&self, pending: Vec<PendingFetch>) {
        for fetch in pending {
            let id = fetch.id;
            let mut events = self.events.clone();
            if fetch.cancelled {
                tokio::spawn(async move {
                    events
                        .send(IoEvent::FetchDone(id, Err(FetchError::Cancelled)))
                        .await
                        .ok();
                });
                continue;
            }
            let (handle, registration) = futures::future::AbortHandle::new_pair();
            self.running.insert(id, handle);
            let running = self.running.clone();
            let fetcher = self.fetcher.clone();
            let domains = self.domains.clone();
            let consent = self.consent.clone();
            let extension_id = self.extension_id.clone();
            tokio::spawn(async move {
                let work = async move {
                    ask_consent(consent, &extension_id, &domains).await?;
                    fetcher.fetch(&domains, fetch.url, fetch.request).await
                };
                let result = futures::future::Abortable::new(work, registration)
                    .await
                    .unwrap_or(Err(FetchError::Cancelled));
                running.remove(id);
                if !running.closed.load(std::sync::atomic::Ordering::SeqCst) {
                    events.send(IoEvent::FetchDone(id, result)).await.ok();
                }
            });
        }
    }
}

impl Drop for NetworkHandle {
    fn drop(&mut self) {
        self.running.cancel_all();
    }
}

/// The broker, when a request is applied (design §0.3 step 3, §7): granted runs, denied fails,
/// `ask` waits for the prompt. Without a broker, T2 is denied.
async fn ask_consent(
    consent: Option<Arc<dyn crate::permissions::T2Consent>>,
    extension_id: &str,
    domains: &DomainList,
) -> Result<(), FetchError> {
    use crate::permissions::{ConsentRequest, ConsentState};
    let Some(consent) = consent else {
        return Err(FetchError::Denied);
    };
    match consent.state(extension_id, CAPABILITY_NETWORK) {
        ConsentState::Granted => Ok(()),
        ConsentState::Denied => Err(FetchError::Denied),
        ConsentState::Ask => {
            let request = ConsentRequest {
                capability: CAPABILITY_NETWORK.into(),
                domains: domains.texts(),
            };
            if consent.request(extension_id, request).await {
                Ok(())
            } else {
                Err(FetchError::Denied)
            }
        }
    }
}

/// A plain HTTP/1.1 server on loopback for the tests: each request gets the response its path
/// names (`respond`), then the connection closes.
#[cfg(test)]
pub(crate) mod test_server {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    pub(crate) struct Server {
        pub port: u16,
        pub connections: Arc<AtomicUsize>,
    }

    impl Server {
        pub(crate) fn connections(&self) -> usize {
            self.connections.load(Ordering::SeqCst)
        }
    }

    /// [`start`] on a thread and tokio runtime of its own, for tests whose runtime may be
    /// replaced (gpui tests re-initialize `gpui_tokio` on every load). It runs until the test
    /// process ends.
    pub(crate) fn start_in_thread(
        respond: impl Fn(&str, u16) -> Option<Vec<u8>> + Send + Sync + 'static,
    ) -> Server {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            runtime.block_on(async move {
                sender.send(start(respond).await).ok();
                futures::future::pending::<()>().await;
            });
        });
        receiver.recv().expect("the server starts")
    }

    /// `respond(path)` gives the raw response bytes, or `None` to keep the connection open
    /// without answering.
    pub(crate) async fn start(
        respond: impl Fn(&str, u16) -> Option<Vec<u8>> + Send + Sync + 'static,
    ) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binds");
        let port = listener.local_addr().expect("address").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let respond = Arc::new(respond);
        tokio::spawn({
            let connections = connections.clone();
            async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    connections.fetch_add(1, Ordering::SeqCst);
                    let respond = respond.clone();
                    tokio::spawn(async move {
                        let mut request = Vec::new();
                        let mut buffer = [0u8; 4096];
                        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                            match stream.read(&mut buffer).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => request.extend_from_slice(&buffer[..n]),
                            }
                        }
                        let text = String::from_utf8_lossy(&request);
                        let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                        match respond(&path, port) {
                            Some(response) => {
                                stream.write_all(&response).await.ok();
                                stream.shutdown().await.ok();
                            }
                            None => tokio::time::sleep(std::time::Duration::from_secs(60)).await,
                        }
                    });
                }
            }
        });
        Server { port, connections }
    }

    pub(crate) fn ok(body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nx-test: yes\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    pub(crate) fn redirect(location: &str) -> Vec<u8> {
        format!("HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\n\r\n")
            .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(patterns: &[&str]) -> DomainList {
        DomainList::parse(&patterns.iter().map(|p| p.to_string()).collect::<Vec<_>>())
            .expect("valid patterns")
    }

    fn request(url: &str) -> FetchRequest {
        FetchRequest {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout_ms: None,
        }
    }

    fn code(result: Result<Url, HostError>) -> Option<HostErrorCode> {
        result.err().map(|error| error.code)
    }

    fn fetcher() -> Fetcher {
        Fetcher::new("probe", Some(Arc::new(PanicResolver)))
    }

    /// `localhost` never goes to DNS: a resolver call fails the test.
    struct PanicResolver;

    impl Resolve for PanicResolver {
        fn resolve(&self, host: &str, _: u16) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> {
            panic!("resolved {host}")
        }
    }

    /// Answers with `first` the first time and `later` every time after, counting the calls.
    struct Rebinding {
        first: Vec<SocketAddr>,
        later: Vec<SocketAddr>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl Resolve for Rebinding {
        fn resolve(&self, _: &str, _: u16) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let addrs = if call == 0 {
                self.first.clone()
            } else {
                self.later.clone()
            };
            async move { Ok(addrs) }.boxed()
        }
    }

    async fn run(
        domains: &DomainList,
        fetcher: &Fetcher,
        request: FetchRequest,
    ) -> Result<FetchResponse, FetchError> {
        let url = check_request(&request, domains).expect("passes the import's checks");
        fetcher.fetch(domains, url, request).await
    }

    #[tokio::test]
    async fn a_local_fetch() {
        let server = test_server::start(|path, _| match path {
            "/hello" => Some(test_server::ok(b"hello")),
            _ => None,
        })
        .await;
        let domains = list(&[&format!("localhost:{}", server.port)]);
        let response = run(
            &domains,
            &fetcher(),
            request(&format!("http://localhost:{}/hello", server.port)),
        )
        .await
        .expect("fetches");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        assert!(response.headers.contains(&("x-test".into(), "yes".into())));
        assert_eq!(
            response.url,
            format!("http://localhost:{}/hello", server.port)
        );
    }

    /// Plan 4.6, done: a 17 MiB response is cut off, by its length or while streaming, and so
    /// is a small gzip body that inflates to 17 MiB.
    #[tokio::test]
    async fn the_body_is_capped_after_decoding() {
        let seventeen = 17usize << 20;
        let bomb = {
            let mut encoder = async_compression::futures::bufread::GzipEncoder::new(
                futures::io::Cursor::new(vec![0u8; seventeen]),
            );
            let mut compressed = Vec::new();
            encoder
                .read_to_end(&mut compressed)
                .await
                .expect("compresses");
            compressed
        };
        assert!(bomb.len() < 1 << 20, "the bomb is small: {}", bomb.len());
        let server = test_server::start(move |path, _| match path {
            "/declared" => {
                Some(format!("HTTP/1.1 200 OK\r\ncontent-length: {seventeen}\r\n\r\n").into_bytes())
            }
            "/streamed" => {
                let mut response = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
                response.extend(std::iter::repeat_n(b'a', seventeen));
                Some(response)
            }
            "/bomb" => {
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: {}\r\n\r\n",
                    bomb.len()
                )
                .into_bytes();
                response.extend_from_slice(&bomb);
                Some(response)
            }
            "/streamed-small" => {
                let mut response = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
                response.extend(std::iter::repeat_n(b'a', 1 << 20));
                Some(response)
            }
            _ => None,
        })
        .await;
        let domains = list(&[&format!("localhost:{}", server.port)]);
        let small = run(
            &domains,
            &fetcher(),
            request(&format!("http://localhost:{}/streamed-small", server.port)),
        )
        .await
        .expect("a 1 MiB body without a length");
        assert_eq!(small.body.len(), 1 << 20);
        for path in ["/declared", "/streamed", "/bomb"] {
            let result = run(
                &domains,
                &fetcher(),
                request(&format!("http://localhost:{}{path}", server.port)),
            )
            .await;
            assert_eq!(result, Err(FetchError::TooLarge), "{path}");
        }
    }

    #[tokio::test]
    async fn redirects_are_checked_hop_by_hop() {
        let server = test_server::start(|path, port| {
            Some(match path {
                "/off-list" => test_server::redirect("https://evil.example/x"),
                "/to-http" => test_server::redirect("http://api.example.com/x"),
                "/other-port" => test_server::redirect(&format!("http://localhost:{}/x", port + 1)),
                "/loop" => test_server::redirect("/loop"),
                "/once" => test_server::redirect("/done"),
                "/done" => test_server::ok(b"done"),
                _ => return None,
            })
        })
        .await;
        let domains = list(&[&format!("localhost:{}", server.port), "api.example.com"]);
        let fetch = |path: &str| {
            let request = request(&format!("http://localhost:{}{path}", server.port));
            let domains = domains.clone();
            async move { run(&domains, &fetcher(), request).await }
        };
        assert_eq!(
            fetch("/off-list").await,
            Err(FetchError::RedirectNotAllowed("evil.example".into()))
        );
        assert_eq!(
            fetch("/to-http").await,
            Err(FetchError::RedirectNotAllowed("api.example.com".into()))
        );
        assert_eq!(
            fetch("/other-port").await,
            Err(FetchError::RedirectNotAllowed(format!(
                "localhost:{}",
                server.port + 1
            )))
        );
        assert!(matches!(
            fetch("/loop").await,
            Err(FetchError::RedirectNotAllowed(_))
        ));
        let before = server.connections();
        assert!(matches!(
            fetch("/loop").await,
            Err(FetchError::RedirectNotAllowed(_))
        ));
        assert_eq!(
            server.connections() - before,
            MAX_REDIRECTS + 1,
            "the first request and five redirects, not a sixth"
        );
        let done = fetch("/once").await.expect("one hop");
        assert_eq!(done.body, b"done");
        assert_eq!(done.url, format!("http://localhost:{}/done", server.port));
    }

    #[tokio::test]
    async fn timeouts() {
        let server = test_server::start(|_, _| None).await;
        let domains = list(&[&format!("localhost:{}", server.port)]);
        let mut slow = request(&format!("http://localhost:{}/", server.port));
        slow.timeout_ms = Some(200);
        assert_eq!(
            run(&domains, &fetcher(), slow).await,
            Err(FetchError::Timeout)
        );
    }

    /// A public domain that resolves to any non-global address is refused before connecting.
    #[tokio::test]
    async fn private_answers_are_refused() {
        for private in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "::ffff:10.0.0.1",
        ] {
            let resolver = Rebinding {
                first: vec![
                    SocketAddr::new("93.184.216.34".parse().unwrap(), 443),
                    SocketAddr::new(private.parse().unwrap(), 443),
                ],
                later: Vec::new(),
                calls: Default::default(),
            };
            let fetcher = Fetcher::new("probe", Some(Arc::new(resolver)));
            let domains = list(&["api.example.com"]);
            let result = run(&domains, &fetcher, request("https://api.example.com/")).await;
            assert!(
                matches!(&result, Err(FetchError::HostNotAllowed(host)) if host.contains(private)),
                "{private}: {result:?}"
            );
        }
    }

    /// DNS rebinding: the name is resolved once and the connection goes to the checked address,
    /// even though a second lookup would say otherwise. Here loopback stands in for a public
    /// address; the server sees one connection (a TLS handshake it cannot answer).
    #[tokio::test]
    async fn connections_go_to_the_checked_address() {
        let server = test_server::start(|_, _| None).await;
        let resolver = Arc::new(Rebinding {
            first: vec![SocketAddr::new(Ipv4Addr::LOCALHOST.into(), server.port)],
            later: vec![SocketAddr::new("10.0.0.1".parse().unwrap(), server.port)],
            calls: Default::default(),
        });
        let fetcher = Fetcher::new("probe", Some(resolver.clone()))
            .with_address_rule(|ip| ip.is_loopback() || is_global(ip));
        let domains = list(&[&format!("api.example.com:{}", server.port)]);
        let mut request = request(&format!("https://api.example.com:{}/", server.port));
        request.timeout_ms = Some(2_000);
        let result = run(&domains, &fetcher, request).await;
        assert!(result.is_err(), "no TLS on the test server: {result:?}");
        assert_eq!(resolver.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(server.connections(), 1);
    }

    #[test]
    fn running_requests_can_be_cancelled() {
        let running = Running::default();
        let (handle, registration) = futures::future::AbortHandle::new_pair();
        running.insert(7, handle);
        assert!(running.cancel(7));
        assert!(!running.cancel(7));
        let aborted = futures::executor::block_on(futures::future::Abortable::new(
            futures::future::pending::<()>(),
            registration,
        ));
        assert!(aborted.is_err());
    }

    #[test]
    fn domain_patterns() {
        let domains = list(&["api.example.com", "*.cdn.example.com", "example.org:8443"]);
        let allowed = |url: &str| code(check_request(&request(url), &domains)).is_none();
        assert!(allowed("https://api.example.com/v1"));
        assert!(
            allowed("https://API.Example.com./v1"),
            "case and trailing dot"
        );
        assert!(allowed("https://a.cdn.example.com/x"));
        assert!(allowed("https://a.b.cdn.example.com/x"), "any depth");
        assert!(allowed("https://example.org:8443/"));
        for refused in [
            "https://cdn.example.com/",
            "https://evilcdn.example.com/",
            "https://api.example.com.evil.net/",
            "https://example.com/",
            "https://api.example.com:8443/",
            "https://example.org/",
            "https://93.184.216.34/",
            // Cyrillic "а" in "аpi": a homograph does not match its ASCII look-alike.
            "https://\u{0430}pi.example.com/",
        ] {
            assert_eq!(
                code(check_request(&request(refused), &domains)),
                Some(HostErrorCode::Denied),
                "{refused}"
            );
        }
        // A UTF-8 pattern matches its punycode form.
        let idn = list(&["bücher.example"]);
        assert!(
            code(check_request(
                &request("https://xn--bcher-kva.example/"),
                &idn
            ))
            .is_none()
        );
    }

    #[test]
    fn patterns_refused_at_load() {
        for bad in [
            "*.com",
            "*.co.uk",
            "*.github.io",
            "com",
            "localhost",
            "127.0.0.1",
            "10.0.0.1",
            "[::1]:80",
            "foo.*.com",
            "example.com:0",
            "example.com:http",
            "",
        ] {
            assert!(
                DomainList::parse(&[bad.to_string()]).is_err(),
                "{bad:?} should be refused"
            );
        }
        assert!(DomainList::parse(&vec!["a.example.com".to_string(); MAX_DOMAINS + 1]).is_err());
        list(&[
            "localhost:8080",
            "127.0.0.1:3000",
            "*.example.com",
            "github.io",
        ]);
    }

    #[test]
    fn import_checks() {
        let domains = list(&["api.example.com", "localhost:8080"]);
        let check = |request: FetchRequest| code(check_request(&request, &domains));
        assert_eq!(
            check(request("https://user:pw@api.example.com/")),
            Some(HostErrorCode::InvalidArgument)
        );
        assert_eq!(
            check(request("ftp://api.example.com/")),
            Some(HostErrorCode::InvalidArgument)
        );
        assert_eq!(
            check(request("not a url")),
            Some(HostErrorCode::InvalidArgument)
        );
        assert_eq!(
            check(request(&format!(
                "https://api.example.com/{}",
                "a".repeat(MAX_URL_BYTES)
            ))),
            Some(HostErrorCode::InvalidArgument)
        );
        assert_eq!(
            check(request("http://api.example.com/")),
            Some(HostErrorCode::Denied)
        );
        assert_eq!(check(request("http://localhost:8080/")), None);
        assert_eq!(check(request("http://127.0.0.1:8080/")), None);
        assert_eq!(
            check(request("http://localhost:8081/")),
            Some(HostErrorCode::Denied)
        );
        for header in [
            "Host",
            "content-length",
            "Transfer-Encoding",
            "Proxy-Authorization",
            "accept-encoding",
        ] {
            let mut forbidden = request("https://api.example.com/");
            forbidden.headers.push((header.into(), "x".into()));
            assert_eq!(
                check(forbidden),
                Some(HostErrorCode::InvalidArgument),
                "{header}"
            );
        }
        let mut ok = request("https://api.example.com/");
        ok.headers.push(("authorization".into(), "Bearer t".into()));
        ok.headers.push(("cookie".into(), "a=b".into()));
        assert_eq!(check(ok.clone()), None, "the guest may send what it holds");
        let mut many = ok.clone();
        many.headers = vec![("x-a".into(), "1".into()); MAX_REQUEST_HEADERS + 1];
        assert_eq!(check(many), Some(HostErrorCode::InvalidArgument));
        let mut big = ok.clone();
        big.body = Some(vec![0; MAX_REQUEST_BODY_BYTES + 1]);
        assert_eq!(check(big), Some(HostErrorCode::InvalidArgument));
        for timeout in [0, 30_001] {
            let mut slow = ok.clone();
            slow.timeout_ms = Some(timeout);
            assert_eq!(check(slow), Some(HostErrorCode::InvalidArgument));
        }
    }

    #[test]
    fn only_global_addresses() {
        for private in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(!is_global(private.parse().unwrap()), "{private}");
        }
        for public in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_global(public.parse().unwrap()), "{public}");
        }
    }
}
