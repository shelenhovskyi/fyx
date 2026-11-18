//! `network:fetch` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §8, plan 4.6, spec decision 23): a
//! host import, not WASI sockets (the WASI context denies those, [`crate::deny_network`]).
//!
//! - [`DomainList`], [`check_request`] and the limits and types are `ui_ext::network`'s (shared
//!   with the Apache harness), re-exported here. An undeclared host is `denied` at the import.
//! - [`Fetcher`]: runs a checked request after the call returns (design §0.3). The host follows
//!   redirects itself, checking every hop like the first. It resolves each name once, refuses
//!   any non-global address for a public domain, and connects only to the addresses it checked
//!   (DNS rebinding). It caps the decoded body while streaming.
//!
//! The import, the counters and the delivery through `handle-io` are in `fyx_ui_v0_3.rs`,
//! `effects.rs` and [`crate::UiExtension::handle_io`].

use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use futures::{
    AsyncReadExt as _, FutureExt as _, SinkExt as _, TryStreamExt as _, future::BoxFuture,
};
pub use ui_ext::network::{
    Allowed, CAPABILITY_NETWORK, DomainList, FetchCounters, FetchError, FetchRequest,
    FetchResponse, IoEvent, MAX_DOMAINS, MAX_IN_FLIGHT, MAX_REDIRECTS, MAX_REQUEST_BODY_BYTES,
    MAX_REQUEST_HEADER_BYTES, MAX_REQUEST_HEADERS, MAX_RESPONSE_BODY_BYTES,
    MAX_RESPONSE_HEADER_BYTES, MAX_STARTED_PER_MINUTE, MAX_TIMEOUT, MAX_URL_BYTES, Method,
    check_request, host_and_port,
};
use ui_ext::v0_2::{HostError, HostErrorCode};

use crate::permissions::{CapabilityCheck, PermissionState};
use url::Url;

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
            .request(reqwest_method(method), url.clone())
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

fn reqwest_method(method: Method) -> reqwest::Method {
    match method {
        Method::Get => reqwest::Method::GET,
        Method::Head => reqwest::Method::HEAD,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Patch => reqwest::Method::PATCH,
        Method::Delete => reqwest::Method::DELETE,
        Method::Options => reqwest::Method::OPTIONS,
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

/// The requests of one instance that started and have not delivered yet, so `cancel-fetch`, a
/// revocation, a reload and a disable can stop them, and the way their outcomes leave.
#[derive(Default)]
pub(crate) struct Running {
    handles: Mutex<HashMap<u64, futures::future::AbortHandle>>,
    /// Aborted because the capability was revoked: they deliver `denied`, not `cancelled`.
    revoked: Mutex<std::collections::HashSet<u64>>,
    /// Where outcomes go; `None` once the instance is gone, so nothing delivers after that and
    /// no task keeps the channel open.
    events: Mutex<Option<futures::channel::mpsc::Sender<IoEvent>>>,
    /// The Extension Inspector's record of each operation: started by the handles, finished here
    /// when its outcome is delivered.
    pub(crate) diagnostics: crate::diagnostics::DiagnosticsBuffer,
}

/// What the Inspector shows of a URL: no user info, query or fragment, which may hold secrets.
pub(crate) fn redacted_url(url: &Url) -> String {
    let mut shown = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        shown.push_str(&format!(":{port}"));
    }
    shown.push_str(url.path());
    shown
}

/// The Inspector's outcome of a finished operation, by its capability and id.
fn io_outcome(event: &IoEvent) -> Option<(&'static str, u64, String)> {
    Some(match event {
        IoEvent::FetchDone(id, Ok(response)) => (
            CAPABILITY_NETWORK,
            *id,
            format!("{} ({} bytes)", response.status, response.body.len()),
        ),
        IoEvent::FetchDone(id, Err(error)) => (CAPABILITY_NETWORK, *id, format!("{error:?}")),
        IoEvent::ProcessExited(exit) => (
            crate::process::CAPABILITY_PROCESS,
            exit.process,
            match (exit.code, exit.killed) {
                (_, true) => "killed".to_string(),
                (Some(code), false) => format!("exited {code}"),
                (None, false) => "ended by a signal".to_string(),
            },
        ),
        IoEvent::SpawnFailed(id, error) => (
            crate::process::CAPABILITY_PROCESS,
            *id,
            format!("not started: {error:?}"),
        ),
        IoEvent::EditDone(id, Ok(())) => (crate::edit::CAPABILITY_EDIT, *id, "applied".into()),
        IoEvent::EditDone(id, Err(error)) => {
            (crate::edit::CAPABILITY_EDIT, *id, format!("{error:?}"))
        }
        _ => return None,
    })
}

impl Running {
    pub(crate) fn new(diagnostics: crate::diagnostics::DiagnosticsBuffer) -> Self {
        Self {
            diagnostics,
            ..Self::default()
        }
    }

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

    /// The capability was revoked: every running request ends with `denied` (decision 26).
    pub(crate) fn revoke_all(&self) {
        let handles: Vec<_> = self.lock().drain().collect();
        let mut revoked = self.revoked.lock().unwrap_or_else(PoisonError::into_inner);
        for (id, handle) in handles {
            revoked.insert(id);
            handle.abort();
        }
    }

    /// The outcome of an aborted request.
    fn aborted(&self, id: u64) -> FetchError {
        if self
            .revoked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id)
        {
            FetchError::Denied
        } else {
            FetchError::Cancelled
        }
    }

    /// The instance is gone (reload, disable, uninstall): every request stops and none
    /// delivers, not even `cancelled` (design §0.3 step 7).
    pub(crate) fn cancel_all(&self) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        for (_, handle) in self.lock().drain() {
            handle.abort();
        }
    }

    /// The way outcomes leave, for a feed that sends without waiting; `None` once the instance
    /// is gone.
    pub(crate) fn sender(&self) -> Option<futures::channel::mpsc::Sender<IoEvent>> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) async fn deliver(&self, event: IoEvent) {
        if let Some((capability, id, outcome)) = io_outcome(&event) {
            self.diagnostics.io_finished(capability, id, outcome);
        }
        let sender = self
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(mut sender) = sender {
            sender.send(event).await.ok();
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
    /// Ids and the in-flight and per-minute caps, shared with the harness.
    counters: FetchCounters,
    pending: Vec<PendingFetch>,
    running: Arc<Running>,
}

impl NetworkState {
    pub(crate) fn new(domains: Arc<DomainList>, running: Arc<Running>) -> Self {
        Self {
            domains,
            counters: FetchCounters::default(),
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
        let id = self.counters.start(std::time::Instant::now())?;
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
                self.counters.finished(fetch.id);
            }
            Vec::new()
        }
    }

    /// Before `handle-io` delivers `fetch-done`: the request is no longer in flight.
    pub(crate) fn delivered(&mut self, id: u64) {
        self.counters.finished(id);
    }
}

/// What a revocation stops besides the running requests (decision 26).
pub(crate) struct Revocable {
    pub(crate) processes: std::sync::Weak<crate::process::ProcessSet>,
    pub(crate) edits: std::sync::Weak<crate::edit::EditSet>,
    pub(crate) observer: std::sync::Weak<crate::observe::ObserveHandle>,
}

/// Starts accepted requests and hands their outcomes to the host, once per instance. Dropping
/// the last [`crate::UiExtension`] clone cancels every running request, so a reloaded or
/// disabled instance gets no outcome of the old one's ids (design §0.3 step 7).
pub(crate) struct NetworkHandle {
    extension_id: Arc<str>,
    domains: Arc<DomainList>,
    fetcher: Fetcher,
    broker: Option<Arc<dyn CapabilityCheck>>,
    running: Arc<Running>,
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
        broker: Option<Arc<dyn CapabilityCheck>>,
        running: Arc<Running>,
    ) -> Self {
        let (events, receiver) = futures::channel::mpsc::channel(MAX_QUEUED_IO_EVENTS);
        *running
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(events);
        Self {
            extension_id,
            domains,
            fetcher,
            broker,
            running,
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
        state(self.broker.as_ref(), &self.extension_id) == PermissionState::Granted
    }

    /// Follows the broker's state changes for this instance, on the current tokio runtime: a
    /// revocation of `network:fetch` ends the running requests with `denied`, and every change
    /// is delivered as `permission-changed`. Only for a component that has `handle-io`.
    pub(crate) fn watch_permissions(&self, revocable: Revocable) {
        let Some(broker) = &self.broker else {
            return;
        };
        let mut changes = broker.changes(&self.extension_id);
        let running = Arc::downgrade(&self.running);
        tokio::spawn(async move {
            use futures::StreamExt as _;
            while let Some((capability, state)) = changes.next().await {
                let Some(running) = running.upgrade() else {
                    return;
                };
                if capability == CAPABILITY_NETWORK && state != PermissionState::Granted {
                    running.revoke_all();
                }
                if state != PermissionState::Granted {
                    match capability.as_str() {
                        crate::process::CAPABILITY_PROCESS => {
                            if let Some(processes) = revocable.processes.upgrade() {
                                processes.kill_all();
                            }
                        }
                        crate::edit::CAPABILITY_EDIT => {
                            if let Some(edits) = revocable.edits.upgrade() {
                                edits.revoke_all();
                            }
                        }
                        crate::observe::CAPABILITY_OBSERVE => {
                            if let Some(observer) = revocable.observer.upgrade() {
                                observer.stop();
                            }
                        }
                        _ => {}
                    }
                }
                running
                    .deliver(IoEvent::PermissionChanged(capability, state))
                    .await;
            }
        });
    }

    /// Starts `pending` on the current tokio runtime, in call order.
    pub(crate) fn start(&self, pending: Vec<PendingFetch>) {
        for fetch in pending {
            let id = fetch.id;
            self.running.diagnostics.io_started(
                CAPABILITY_NETWORK,
                id,
                format!("{:?} {}", fetch.request.method, redacted_url(&fetch.url)),
            );
            let running = self.running.clone();
            if fetch.cancelled {
                tokio::spawn(async move {
                    running
                        .deliver(IoEvent::FetchDone(id, Err(FetchError::Cancelled)))
                        .await;
                });
                continue;
            }
            let (handle, registration) = futures::future::AbortHandle::new_pair();
            self.running.insert(id, handle);
            let fetcher = self.fetcher.clone();
            let domains = self.domains.clone();
            let broker = self.broker.clone();
            let extension_id = self.extension_id.clone();
            tokio::spawn(async move {
                let work = async move {
                    ask_consent(broker.as_ref(), &extension_id).await?;
                    fetcher.fetch(&domains, fetch.url, fetch.request).await
                };
                let result = match futures::future::Abortable::new(work, registration).await {
                    Ok(result) => result,
                    Err(_) => Err(running.aborted(id)),
                };
                running.remove(id);
                running.deliver(IoEvent::FetchDone(id, result)).await;
            });
        }
    }
}

impl Drop for NetworkHandle {
    fn drop(&mut self) {
        self.running.cancel_all();
    }
}

fn state(broker: Option<&Arc<dyn CapabilityCheck>>, extension_id: &str) -> PermissionState {
    broker.map_or(PermissionState::Unavailable, |broker| {
        broker.state(extension_id, CAPABILITY_NETWORK)
    })
}

/// The broker, when a request is applied (design §0.3 step 3, §7): granted runs, `ask` waits
/// for the prompt, anything else fails with `denied`. Without a broker, T2 is denied.
async fn ask_consent(
    broker: Option<&Arc<dyn CapabilityCheck>>,
    extension_id: &str,
) -> Result<(), FetchError> {
    match state(broker, extension_id) {
        PermissionState::Granted => Ok(()),
        PermissionState::Ask => match broker {
            Some(broker) => match broker.request(extension_id, CAPABILITY_NETWORK).await {
                PermissionState::Granted => Ok(()),
                _ => Err(FetchError::Denied),
            },
            None => Err(FetchError::Denied),
        },
        PermissionState::Denied | PermissionState::Unavailable => Err(FetchError::Denied),
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
    #[test]
    fn the_inspector_never_shows_url_secrets() {
        let url =
            super::Url::parse("https://user:pw@api.example.com:8443/v1/items?token=s3cret#frag")
                .unwrap();
        assert_eq!(
            super::redacted_url(&url),
            "https://api.example.com:8443/v1/items"
        );
    }
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
