//! The pure part of `network:fetch` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §8, plan 4.6):
//! the limits, the request and outcome types, the manifest's domain list and the checks the
//! import makes at once. No I/O and no GPL code, so the Apache harness (`fyx_ext_harness`) makes
//! the same checks as the host. The engine (resolution, address rules, redirects, the decoded
//! body cap) is the host's, in `ui_ext_wasm::network`.

use std::{
    collections::{BTreeSet, VecDeque},
    fmt,
    time::{Duration, Instant},
};

use url::Url;

use crate::v0_2::{HostError, HostErrorCode};

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
    /// A T2 capability's state changed (the prompt was answered, or the extensions page
    /// changed it); in-flight work of a revoked one was cancelled first.
    PermissionChanged(String, PermissionState),
    /// `process:spawn`: up to 64 KiB of a process's output, in order.
    ProcessOutput(crate::process::ProcessOutput),
    /// After its last output; exactly one per started process.
    ProcessExited(crate::process::ProcessExit),
    /// Instead of any output and exit: the process never started.
    SpawnFailed(u64, crate::process::SpawnError),
    /// `workspace:edit`: the outcome of one edit, applied all or nothing.
    EditDone(u64, Result<(), crate::edit::EditError>),
    /// `agent:observe`: one event, after `start`, beginning with a snapshot.
    Agent(crate::observe::AgentEvent),
    /// fyx:ui 0.4: the extension's terminals and, with `terminal:observe`, everyone's.
    Terminal(crate::terminal::TerminalEvent),
    /// fyx:ui 0.4 `ui:approvals`: the card `id` was answered, or closed unanswered (`None`).
    ApprovalAnswered(u64, Option<crate::approvals::ApprovalOption>),
}

/// The state of a T2 capability for one extension: the WIT `permission-state` (design.md §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    /// Allowed by the user.
    Granted,
    /// Refused or revoked by the user; only the extensions page changes it back.
    Denied,
    /// Not asked yet: the first use shows the prompt.
    Ask,
    /// Not available here: not declared, or (later) the worktree is not trusted.
    Unavailable,
}

/// A host name pattern: `example.com` (that host only) or `*.example.com` (any subdomain at any
/// depth, not `example.com` itself). Compared after IDNA (punycode), lowercasing and dropping a
/// trailing dot. IP literals and `localhost` are refused.
#[derive(Clone, Debug, PartialEq, Eq)]
enum HostPattern {
    Exact(String),
    Subdomain(String),
}

impl HostPattern {
    fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        let (subdomain, rest) = match input.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, input),
        };
        if rest.contains('*') {
            return Err("a wildcard is allowed only as the leading `*.`".into());
        }
        let rest = rest.strip_suffix('.').unwrap_or(rest);
        if rest.is_empty() || rest.contains("..") || rest.starts_with('.') {
            return Err("not a host name".into());
        }
        let host = match url::Host::parse(rest) {
            Ok(url::Host::Domain(host)) => host.to_ascii_lowercase(),
            Ok(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => {
                return Err("an IP address is not a host name pattern".into());
            }
            Err(error) => return Err(format!("not a host name: {error}")),
        };
        if host == "localhost" || host.ends_with(".localhost") {
            return Err("localhost needs a port, such as localhost:8080".into());
        }
        Ok(if subdomain {
            HostPattern::Subdomain(host)
        } else {
            HostPattern::Exact(host)
        })
    }

    /// `host` as `Url` gives it: already punycode and lowercase, maybe with a trailing dot.
    fn matches(&self, host: &str) -> bool {
        let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
        match self {
            HostPattern::Exact(pattern) => host == *pattern,
            HostPattern::Subdomain(base) => host
                .strip_suffix(base.as_str())
                .and_then(|prefix| prefix.strip_suffix('.'))
                .is_some_and(|label| !label.is_empty()),
        }
    }
}

/// RFC 9110 `token`.
fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

/// RFC 9110 `field-value`: visible characters, spaces and tabs, and obs-text; no CR, LF or NUL.
fn is_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte == b'\t' || byte == b' ' || (0x21..=0x7e).contains(&byte) || byte >= 0x80)
}

/// The import's in-flight and per-minute caps ([`MAX_IN_FLIGHT`], [`MAX_STARTED_PER_MINUTE`]).
/// A request counts as in flight from the import until its `fetch-done` is delivered.
#[derive(Debug, Default)]
pub struct FetchCounters {
    next_id: u64,
    in_flight: BTreeSet<u64>,
    started: VecDeque<Instant>,
}

impl FetchCounters {
    /// Admits one request at `now` and returns its id (never repeated), or `limit-exceeded`.
    pub fn start(&mut self, now: Instant) -> Result<u64, HostError> {
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return Err(HostError {
                code: HostErrorCode::LimitExceeded,
                message: format!("at most {MAX_IN_FLIGHT} requests in flight"),
            });
        }
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
        self.next_id += 1;
        self.in_flight.insert(self.next_id);
        self.started.push_back(now);
        Ok(self.next_id)
    }

    /// The request's outcome was delivered, or the call that made it failed.
    pub fn finished(&mut self, id: u64) {
        self.in_flight.remove(&id);
    }
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
pub struct Allowed {
    /// A declared `localhost`/`127.0.0.1` port: `http://` allowed, never resolved through DNS.
    pub loopback: bool,
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
    pub fn parse(patterns: &[String]) -> Result<Self, String> {
        if patterns.len() > MAX_DOMAINS {
            return Err(format!(
                "at most {MAX_DOMAINS} network domains, got {}",
                patterns.len()
            ));
        }
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

    /// The pattern that allows `url`'s host and port, if any.
    pub fn allows(&self, url: &Url) -> Option<Allowed> {
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

fn parse_pattern(text: &str) -> Result<DomainPattern, String> {
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            let port: u16 = port
                .parse()
                .map_err(|_| format!("network domain {text:?}: {port:?} is not a port"))?;
            if port == 0 {
                return Err(format!("network domain {text:?}: port 0"));
            }
            (host, Some(port))
        }
        _ => (text, None),
    };
    if host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" {
        if port.is_none() {
            return Err(format!(
                "network domain {text:?}: localhost needs a port, such as localhost:8080"
            ));
        }
        return Ok(DomainPattern {
            text: text.to_string(),
            host: PatternHost::Loopback,
            port,
        });
    }
    let pattern =
        HostPattern::parse(host).map_err(|error| format!("network domain {text:?}: {error}"))?;
    // A wildcard over a public suffix (`*.com`, `*.github.io`) would allow every site under it;
    // an exact host that is one is a site of its own (`github.io`), unless it is a bare label.
    match &pattern {
        HostPattern::Subdomain(base) if is_public_suffix(base) => {
            return Err(format!(
                "network domain {text:?}: {base} is a public suffix, so the pattern would allow \
                 every site under it"
            ));
        }
        HostPattern::Exact(host) if !host.contains('.') => {
            return Err(format!(
                "network domain {text:?}: {host} is a top-level domain, not a host"
            ));
        }
        _ => {}
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
        if !is_header_name(name) {
            return Err(invalid(format!("bad header name {name:?}")));
        }
        if !is_header_value(value) {
            return Err(invalid(format!("bad value for header {name}")));
        }
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

/// `host:port` for messages, `host` when the port is the scheme's default.
pub fn host_and_port(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        _ => url.to_string(),
    }
}

/// Public suffixes under which anyone can register a site: every single label (`com`, `io`), and
/// these multi-label ones from the Public Suffix List, a snapshot of the widely used ICANN and
/// private entries. A `network_domains` entry may be neither one of them nor a wildcard over one.
const MULTI_LABEL_PUBLIC_SUFFIXES: &[&str] = &[
    // ICANN second-level registries.
    "ac.uk",
    "co.uk",
    "gov.uk",
    "ltd.uk",
    "me.uk",
    "net.uk",
    "nhs.uk",
    "org.uk",
    "plc.uk",
    "sch.uk",
    "com.au",
    "edu.au",
    "gov.au",
    "net.au",
    "org.au",
    "co.nz",
    "net.nz",
    "org.nz",
    "co.jp",
    "ne.jp",
    "or.jp",
    "ac.jp",
    "go.jp",
    "co.kr",
    "or.kr",
    "com.br",
    "net.br",
    "org.br",
    "com.cn",
    "net.cn",
    "org.cn",
    "gov.cn",
    "com.hk",
    "com.tw",
    "com.sg",
    "com.my",
    "co.in",
    "net.in",
    "org.in",
    "co.id",
    "or.id",
    "com.mx",
    "com.ar",
    "com.co",
    "com.tr",
    "com.ua",
    "org.ua",
    "co.za",
    "org.za",
    "com.pl",
    "com.ru",
    "org.ru",
    "co.il",
    "org.il",
    "com.eg",
    "com.sa",
    "com.pk",
    "com.ph",
    "com.vn",
    "com.ng",
    "co.ke",
    // Private entries: hosting platforms where any customer gets a subdomain.
    "github.io",
    "githubusercontent.com",
    "gitlab.io",
    "herokuapp.com",
    "vercel.app",
    "netlify.app",
    "pages.dev",
    "workers.dev",
    "web.app",
    "firebaseapp.com",
    "appspot.com",
    "blogspot.com",
    "azurewebsites.net",
    "cloudapp.net",
    "cloudfront.net",
    "amazonaws.com",
    "s3.amazonaws.com",
    "elasticbeanstalk.com",
    "onrender.com",
    "fly.dev",
    "repl.co",
    "glitch.me",
    "ngrok.io",
    "ngrok-free.app",
    "trycloudflare.com",
    "deno.dev",
    "surge.sh",
];

/// Whether `domain` (lowercase ASCII, no trailing dot) is a public suffix: a single label, or
/// one of [`MULTI_LABEL_PUBLIC_SUFFIXES`]. Also used by the `network:fetch` domain list in
/// `ui_ext_wasm`, which refuses a wildcard whose base is one.
pub fn is_public_suffix(domain: &str) -> bool {
    !domain.contains('.') || MULTI_LABEL_PUBLIC_SUFFIXES.contains(&domain)
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
    fn counters() {
        let mut counters = FetchCounters::default();
        let now = Instant::now();
        let ids: Vec<u64> = (0..MAX_IN_FLIGHT)
            .map(|_| counters.start(now).expect("room"))
            .collect();
        assert_eq!(ids, (1..=MAX_IN_FLIGHT as u64).collect::<Vec<_>>());
        assert_eq!(
            counters.start(now).unwrap_err().code,
            HostErrorCode::LimitExceeded
        );
        counters.finished(1);
        assert_eq!(
            counters.start(now),
            Ok(MAX_IN_FLIGHT as u64 + 1),
            "ids never repeat"
        );
        for id in 2..=MAX_IN_FLIGHT as u64 + 1 {
            counters.finished(id);
        }
        let mut started = MAX_IN_FLIGHT + 1;
        while started < MAX_STARTED_PER_MINUTE {
            let id = counters.start(now).expect("under the rate");
            counters.finished(id);
            started += 1;
        }
        assert_eq!(
            counters.start(now).unwrap_err().code,
            HostErrorCode::LimitExceeded,
            "121st in a minute"
        );
        assert!(counters.start(now + Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn header_syntax() {
        assert!(is_header_name("x-api-key"));
        assert!(!is_header_name("bad name"));
        assert!(!is_header_name(""));
        assert!(is_header_value("Bearer a.b-c"));
        assert!(!is_header_value("a\r\nInjected: 1"));
    }
}
