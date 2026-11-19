//! fyx:ui 0.4 loopback listeners and the MCP wrapper (`network:listen`, design wit-0.4 §B.7,
//! plan 5.11).
//!
//! The host listens on `127.0.0.1:0`; the extension handles messages. Before the extension sees
//! anything, a connection must present the listener's host token (its `auth`), carry no `Origin`
//! (a browser page cannot reach the listener) and name `127.0.0.1:<port>` or `localhost:<port>`
//! as its `Host` (DNS rebinding). The `mcp-*` kinds speak JSON-RPC 2.0 and MCP themselves:
//! `initialize`, `ping`, `tools/list` from the registered tools, and `tools/call` as
//! [`ListenerEvent::McpCall`], answered with [`ListenerSet::respond`]. A call stays open until
//! answered and is cancelled with its connection. `mcp-websocket` answers the `mcp` subprotocol
//! (Claude Code's IDE connection); `mcp-streamable-http` serves `/mcp` with sessions and a
//! server-sent event stream for notifications (Gemini CLI's).

use std::{
    collections::HashMap,
    net::Ipv4Addr,
    sync::{
        Arc, Mutex, PoisonError, Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_tungstenite::tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
    http,
    protocol::WebSocketConfig,
};
use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
    future::{AbortHandle, Abortable},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};
use ui_ext::{
    integration::{
        DiscoveryFileSpec, ListenerEvent, ListenerObserver, ListenerProtocol, ListenerSpec,
        MAX_TOOLS, McpCall, McpError, OpenListener, ToolDef, is_tool_name,
    },
    v0_2::{HostError, HostErrorCode},
};

/// Open connections per listener: websocket connections, HTTP connections and MCP sessions each.
pub(crate) const MAX_CONNECTIONS: usize = 16;
/// A message, a request body, an answer or a tool's schema.
pub(crate) const MAX_MESSAGE_BYTES: usize = 4 << 20;
/// Per connection; one more closes it.
pub(crate) const MAX_MESSAGES_PER_SECOND: u32 = 100;
/// `mcp-streamable-http` serves only this path.
pub(crate) const MCP_PATH: &str = "/mcp";
/// What Claude Code asks for on its IDE websocket.
pub(crate) const MCP_SUBPROTOCOL: &str = "mcp";
/// Supported MCP revisions, newest first; `initialize` echoes the client's when it is one.
pub(crate) const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const SESSION_HEADER: &str = "mcp-session-id";
const MAX_HEAD_BYTES: usize = 16 << 10;
const MAX_NOTIFICATION_METHOD_BYTES: usize = 128;
const MAX_TOOL_DESCRIPTION_BYTES: usize = 16 << 10;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

fn host_error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

fn invalid(message: impl Into<String>) -> HostError {
    host_error(HostErrorCode::InvalidArgument, message)
}

/// A random host token: 256 bits, hex.
fn new_token() -> std::io::Result<String> {
    use rand::TryRngCore as _;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(std::io::Error::other)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn is_mcp(protocol: ListenerProtocol) -> bool {
    matches!(
        protocol,
        ListenerProtocol::McpWebsocket | ListenerProtocol::McpStreamableHttp
    )
}

/// Why a connection is refused before the extension sees it: an HTTP status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Refused(pub(crate) u16);

/// The §B.7 checks on a request head. `header` gives every value of a header (lower-case name).
pub(crate) fn admit<'a>(
    spec: &ListenerSpec,
    token: &str,
    port: u16,
    target: &str,
    header: &dyn Fn(&str) -> Vec<&'a str>,
) -> Result<(), Refused> {
    let one = |name: &str| -> Result<Option<&'a str>, Refused> {
        let values = header(name);
        match values.as_slice() {
            [] => Ok(None),
            [value] => Ok(Some(*value)),
            _ => Err(Refused(400)),
        }
    };
    if !header("origin").is_empty() {
        return Err(Refused(403));
    }
    let host = one("host")?
        .ok_or(Refused(403))?
        .trim()
        .to_ascii_lowercase();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Err(Refused(403));
    }
    let presented = if let Some(name) = &spec.auth.header {
        one(&name.to_ascii_lowercase())?.map(str::trim)
    } else if spec.auth.bearer {
        one("authorization")?.and_then(|value| {
            let (scheme, rest) = value.trim().split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
        })
    } else if let Some(name) = &spec.auth.query {
        let query = target.split_once('?').map_or("", |(_, query)| query);
        let mut values = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value);
        let value = values.next();
        if values.next().is_some() {
            return Err(Refused(400));
        }
        value
    } else {
        None
    };
    match presented {
        Some(presented) if constant_time_eq(presented.as_bytes(), token.as_bytes()) => Ok(()),
        _ => Err(Refused(401)),
    }
}

/// At most [`MAX_MESSAGES_PER_SECOND`] in any one-second window.
struct RateLimit {
    window: Instant,
    count: u32,
}

impl RateLimit {
    fn new() -> Self {
        Self {
            window: Instant::now(),
            count: 0,
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.count = 0;
        }
        self.count += 1;
        self.count <= MAX_MESSAGES_PER_SECOND
    }
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const REQUEST_CANCELLED: i32 = -32800;

/// One client connection: a websocket, or an MCP session of `mcp-streamable-http`.
struct Connection {
    /// Text to send: the websocket's writer, or the session's open event stream.
    outbound: Option<(u64, mpsc::UnboundedSender<String>)>,
    /// `mcp-streamable-http`: the `Mcp-Session-Id`.
    session: Option<String>,
}

struct Listener {
    spec: ListenerSpec,
    port: u16,
    token: String,
    tools: Mutex<Vec<ToolDef>>,
    connections: Mutex<HashMap<u64, Connection>>,
    /// The accept loop and every connection task, aborted when the listener closes.
    tasks: Mutex<HashMap<u64, AbortHandle>>,
    open_sockets: AtomicUsize,
    discovery_files: Mutex<Vec<DiscoveryFileSpec>>,
}

impl Listener {
    fn send_to(&self, connection: Option<u64>, text: &str) -> Result<(), HostError> {
        let connections = self
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match connection {
            Some(id) => {
                let connection = connections
                    .get(&id)
                    .ok_or_else(|| invalid(format!("no open connection {id}")))?;
                if let Some((_, sender)) = &connection.outbound {
                    sender.unbounded_send(text.to_owned()).ok();
                }
            }
            None => {
                for (_, sender) in connections.values().filter_map(|c| c.outbound.as_ref()) {
                    sender.unbounded_send(text.to_owned()).ok();
                }
            }
        }
        Ok(())
    }

    fn open_listener(&self) -> OpenListener {
        OpenListener {
            name: self.spec.name.clone(),
            protocol: self.spec.protocol,
            port: self.port,
            token: self.token.clone(),
            discovery_files: self
                .discovery_files
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }
}

struct PendingCall {
    listener: String,
    connection: u64,
    request_id: Value,
    reply: oneshot::Sender<Result<Value, McpError>>,
}

struct Inner {
    extension_id: Arc<str>,
    sink: Arc<dyn Fn(ListenerEvent) + Send + Sync>,
    observer: Option<Arc<dyn ListenerObserver>>,
    next_id: AtomicU64,
    listeners: Mutex<HashMap<String, Arc<Listener>>>,
    calls: Mutex<HashMap<u64, PendingCall>>,
    requests: Mutex<HashMap<u64, oneshot::Sender<(u16, String)>>>,
}

impl Inner {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn listener(&self, name: &str) -> Result<Arc<Listener>, HostError> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
            .ok_or_else(|| {
                host_error(
                    HostErrorCode::Unavailable,
                    format!("listener {name:?} is not open"),
                )
            })
    }

    /// Cancels the open calls `matches` picks, each with one `mcp-cancelled`.
    fn cancel_calls(&self, matches: impl Fn(&PendingCall) -> bool) {
        let cancelled: Vec<u64> = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            let ids: Vec<u64> = calls
                .iter()
                .filter(|(_, call)| matches(call))
                .map(|(id, _)| *id)
                .collect();
            for id in &ids {
                calls.remove(id);
            }
            ids
        };
        for id in cancelled {
            (self.sink)(ListenerEvent::McpCancelled(id));
        }
    }

    fn end_connection(&self, listener: &Listener, connection: u64) {
        listener
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&connection);
        let name = listener.spec.name.as_str();
        self.cancel_calls(|call| call.listener == name && call.connection == connection);
    }

    /// One JSON-RPC message of an `mcp-*` connection.
    fn handle_mcp(&self, listener: &Listener, connection: u64, message: Value) -> Handled {
        let Value::Object(message) = message else {
            return Handled::Reply(rpc_error(
                &Value::Null,
                INVALID_REQUEST,
                "batches and non-objects are not supported",
            ));
        };
        let id = message.get("id").cloned();
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return match id {
                Some(id) => Handled::Reply(rpc_error(&id, INVALID_REQUEST, "not JSON-RPC 2.0")),
                None => Handled::Accepted,
            };
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to a request the host never sends.
            return Handled::Accepted;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            if method == "notifications/cancelled"
                && let Some(request_id) = params.get("requestId")
            {
                let name = listener.spec.name.as_str();
                self.cancel_calls(|call| {
                    call.listener == name
                        && call.connection == connection
                        && &call.request_id == request_id
                });
            }
            return Handled::Accepted;
        };
        match method {
            "initialize" => {
                let requested = params.get("protocolVersion").and_then(Value::as_str);
                let version = requested
                    .filter(|requested| PROTOCOL_VERSIONS.contains(requested))
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                Handled::Reply(rpc_result(
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": true } },
                        "serverInfo": {
                            "name": &*self.extension_id,
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                ))
            }
            "ping" => Handled::Reply(rpc_result(&id, json!({}))),
            "tools/list" => {
                let tools = listener
                    .tools
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "inputSchema": serde_json::from_str::<Value>(&tool.input_schema)
                                .unwrap_or_else(|_| json!({ "type": "object" })),
                        })
                    })
                    .collect::<Vec<_>>();
                Handled::Reply(rpc_result(&id, json!({ "tools": tools })))
            }
            "tools/call" => {
                let Some(tool) = params.get("name").and_then(Value::as_str) else {
                    return Handled::Reply(rpc_error(&id, INVALID_PARAMS, "no tool name"));
                };
                let known = listener
                    .tools
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .iter()
                    .any(|registered| registered.name == tool);
                if !known {
                    return Handled::Reply(rpc_error(
                        &id,
                        INVALID_PARAMS,
                        &format!("unknown tool: {tool}"),
                    ));
                }
                let arguments = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(arguments @ Value::Object(_)) => arguments.clone(),
                    Some(_) => {
                        return Handled::Reply(rpc_error(
                            &id,
                            INVALID_PARAMS,
                            "arguments must be an object",
                        ));
                    }
                };
                let call_id = self.next_id();
                let (reply, receiver) = oneshot::channel();
                self.calls
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(
                        call_id,
                        PendingCall {
                            listener: listener.spec.name.clone(),
                            connection,
                            request_id: id.clone(),
                            reply,
                        },
                    );
                (self.sink)(ListenerEvent::McpCall(McpCall {
                    call_id,
                    listener: listener.spec.name.clone(),
                    connection,
                    tool: tool.to_owned(),
                    arguments: arguments.to_string(),
                }));
                Handled::Call {
                    request_id: id,
                    receiver,
                }
            }
            _ => Handled::Reply(rpc_error(
                &id,
                METHOD_NOT_FOUND,
                &format!("method not found: {method}"),
            )),
        }
    }
}

enum Handled {
    Reply(Value),
    /// A notification or a response: nothing to answer.
    Accepted,
    Call {
        request_id: Value,
        receiver: oneshot::Receiver<Result<Value, McpError>>,
    },
}

fn call_answer(
    request_id: &Value,
    answer: Result<Result<Value, McpError>, oneshot::Canceled>,
) -> Value {
    match answer {
        Ok(Ok(result)) => rpc_result(request_id, result),
        Ok(Err(error)) => rpc_error(request_id, error.code, &error.message),
        Err(oneshot::Canceled) => rpc_error(request_id, REQUEST_CANCELLED, "request cancelled"),
    }
}

/// Removes a connection task's abort handle and its connection when the task ends, aborted or
/// not, cancelling the connection's open calls.
struct TaskGuard {
    inner: Weak<Inner>,
    listener: Weak<Listener>,
    task: u64,
    connection: Option<u64>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        let Some(listener) = self.listener.upgrade() else {
            return;
        };
        listener
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.task);
        listener.open_sockets.fetch_sub(1, Ordering::Relaxed);
        if let (Some(connection), Some(inner)) = (self.connection, self.inner.upgrade()) {
            inner.end_connection(&listener, connection);
        }
    }
}

/// One instance's listeners. Dropping it closes them all.
pub(crate) struct ListenerSet {
    inner: Arc<Inner>,
}

impl ListenerSet {
    pub(crate) fn new(
        extension_id: Arc<str>,
        sink: Arc<dyn Fn(ListenerEvent) + Send + Sync>,
        observer: Option<Arc<dyn ListenerObserver>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                extension_id,
                sink,
                observer,
                next_id: AtomicU64::new(1),
                listeners: Mutex::default(),
                calls: Mutex::default(),
                requests: Mutex::default(),
            }),
        }
    }

    /// Opens `spec` on `127.0.0.1:0` and starts accepting on `runtime`; an open one keeps its
    /// port and token and only takes the new discovery files. The observer hears of both.
    pub(crate) fn open(
        &self,
        runtime: &tokio::runtime::Handle,
        spec: &ListenerSpec,
        discovery_files: Vec<DiscoveryFileSpec>,
    ) -> std::io::Result<u16> {
        if let Ok(open) = self.inner.listener(&spec.name) {
            *open
                .discovery_files
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = discovery_files;
            if let Some(observer) = &self.inner.observer {
                observer.opened(&self.inner.extension_id, &open.open_listener());
            }
            return Ok(open.port);
        }
        let socket = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        socket.set_nonblocking(true)?;
        let port = socket.local_addr()?.port();
        let socket = {
            let _runtime = runtime.enter();
            tokio::net::TcpListener::from_std(socket)?
        };
        let listener = Arc::new(Listener {
            spec: spec.clone(),
            port,
            token: new_token()?,
            tools: Mutex::default(),
            connections: Mutex::default(),
            tasks: Mutex::default(),
            open_sockets: AtomicUsize::new(0),
            discovery_files: Mutex::new(discovery_files),
        });
        let accept_id = self.inner.next_id();
        let (abort, registration) = AbortHandle::new_pair();
        listener
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(accept_id, abort);
        let inner = Arc::downgrade(&self.inner);
        let weak_listener = Arc::downgrade(&listener);
        let runtime_for_tasks = runtime.clone();
        runtime.spawn(Abortable::new(
            accept_loop(socket, inner, weak_listener, runtime_for_tasks),
            registration,
        ));
        self.inner
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(spec.name.clone(), listener.clone());
        if let Some(observer) = &self.inner.observer {
            observer.opened(&self.inner.extension_id, &listener.open_listener());
        }
        Ok(port)
    }

    /// Closes the listener (revocation, reload, disable, uninstall): its connections drop, its
    /// open calls are cancelled and the observer removes its discovery files.
    pub(crate) fn close(&self, name: &str) {
        let Some(listener) = self
            .inner
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name)
        else {
            return;
        };
        let tasks: Vec<AbortHandle> = listener
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .map(|(_, task)| task)
            .collect();
        for task in tasks {
            task.abort();
        }
        listener
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.inner.cancel_calls(|call| call.listener == name);
        if let Some(observer) = &self.inner.observer {
            observer.closed(&self.inner.extension_id, name);
        }
    }

    pub(crate) fn close_all(&self) {
        let names: Vec<String> = self
            .inner
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        for name in names {
            self.close(&name);
        }
    }

    pub(crate) fn is_open(&self, name: &str) -> bool {
        self.inner.listener(name).is_ok()
    }

    /// `${listen.<name>.port}`.
    pub(crate) fn port(&self, name: &str) -> Option<u16> {
        self.inner.listener(name).ok().map(|listener| listener.port)
    }

    /// `${listen.<name>.token}`; the caller checks `token_in_env`.
    pub(crate) fn token(&self, name: &str) -> Option<String> {
        self.inner
            .listener(name)
            .ok()
            .map(|listener| listener.token.clone())
    }

    /// `listeners.register-tools`: replaces the listener's tools; open connections hear
    /// `notifications/tools/list_changed`.
    pub(crate) fn register_tools(&self, name: &str, tools: Vec<ToolDef>) -> Result<(), HostError> {
        let listener = self.inner.listener(name)?;
        if !is_mcp(listener.spec.protocol) {
            return Err(invalid(format!("listener {name:?} does not speak MCP")));
        }
        if tools.len() > MAX_TOOLS {
            return Err(host_error(
                HostErrorCode::LimitExceeded,
                format!("{} tools, at most {MAX_TOOLS}", tools.len()),
            ));
        }
        for (index, tool) in tools.iter().enumerate() {
            if !is_tool_name(&tool.name) {
                return Err(invalid(format!(
                    "tool name {:?} must be [A-Za-z0-9_-]{{1,64}}",
                    tool.name
                )));
            }
            if tools[..index].iter().any(|other| other.name == tool.name) {
                return Err(invalid(format!("tool {:?} is registered twice", tool.name)));
            }
            if tool.description.len() > MAX_TOOL_DESCRIPTION_BYTES
                || tool.input_schema.len() > MAX_MESSAGE_BYTES
            {
                return Err(host_error(
                    HostErrorCode::LimitExceeded,
                    format!("tool {:?} is too large", tool.name),
                ));
            }
            match serde_json::from_str::<Value>(&tool.input_schema) {
                Ok(Value::Object(_)) => {}
                _ => {
                    return Err(invalid(format!(
                        "tool {:?}: the input schema must be a JSON object",
                        tool.name
                    )));
                }
            }
        }
        let changed = {
            let mut registered = listener
                .tools
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let changed = *registered != tools;
            *registered = tools;
            changed
        };
        if changed {
            let notification =
                json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" });
            listener.send_to(None, &notification.to_string())?;
        }
        Ok(())
    }

    /// `listeners.respond`: the answer to an open call, a full `CallToolResult` as JSON text, or
    /// a JSON-RPC error.
    pub(crate) fn respond(
        &self,
        call_id: u64,
        outcome: Result<String, McpError>,
    ) -> Result<(), HostError> {
        let outcome = match outcome {
            Ok(text) => {
                if text.len() > MAX_MESSAGE_BYTES {
                    return Err(host_error(
                        HostErrorCode::LimitExceeded,
                        format!("the result is more than {MAX_MESSAGE_BYTES} bytes"),
                    ));
                }
                match serde_json::from_str::<Value>(&text) {
                    Ok(result @ Value::Object(_)) => Ok(result),
                    _ => return Err(invalid("the result must be a JSON object")),
                }
            }
            Err(error) => Err(error),
        };
        let call = self
            .inner
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&call_id)
            .ok_or_else(|| invalid(format!("no open call {call_id}")))?;
        call.reply.send(outcome).ok();
        Ok(())
    }

    /// `listeners.notify`: an MCP notification to one connection or to all.
    pub(crate) fn notify(
        &self,
        name: &str,
        connection: Option<u64>,
        method: &str,
        params: &str,
    ) -> Result<(), HostError> {
        let listener = self.inner.listener(name)?;
        if !is_mcp(listener.spec.protocol) {
            return Err(invalid(format!("listener {name:?} does not speak MCP")));
        }
        if method.is_empty() || method.len() > MAX_NOTIFICATION_METHOD_BYTES {
            return Err(invalid("the method must be 1 to 128 bytes"));
        }
        if params.len() > MAX_MESSAGE_BYTES {
            return Err(host_error(
                HostErrorCode::LimitExceeded,
                format!("the params are more than {MAX_MESSAGE_BYTES} bytes"),
            ));
        }
        let params: Value =
            serde_json::from_str(params).map_err(|_| invalid("the params must be JSON"))?;
        if !params.is_object() && !params.is_array() {
            return Err(invalid("the params must be a JSON object or array"));
        }
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        listener.send_to(connection, &message.to_string())
    }

    /// `listeners.send`: a text frame on a `websocket` listener's connection.
    pub(crate) fn send(&self, name: &str, connection: u64, text: &str) -> Result<(), HostError> {
        let listener = self.inner.listener(name)?;
        if listener.spec.protocol != ListenerProtocol::Websocket {
            return Err(invalid(format!(
                "listener {name:?} is not a websocket listener"
            )));
        }
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(host_error(
                HostErrorCode::LimitExceeded,
                format!("the text is more than {MAX_MESSAGE_BYTES} bytes"),
            ));
        }
        listener.send_to(Some(connection), text)
    }

    /// `listeners.reply`: the answer to an `http` listener's request.
    pub(crate) fn reply(
        &self,
        request_id: u64,
        status: u16,
        body: String,
    ) -> Result<(), HostError> {
        if !(200..=599).contains(&status) {
            return Err(invalid("the status must be 200 to 599"));
        }
        if body.len() > MAX_MESSAGE_BYTES {
            return Err(host_error(
                HostErrorCode::LimitExceeded,
                format!("the body is more than {MAX_MESSAGE_BYTES} bytes"),
            ));
        }
        let sender = self
            .inner
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&request_id)
            .ok_or_else(|| invalid(format!("no open request {request_id}")))?;
        sender.send((status, body)).ok();
        Ok(())
    }
}

impl Drop for ListenerSet {
    fn drop(&mut self) {
        self.close_all();
    }
}

async fn accept_loop(
    socket: tokio::net::TcpListener,
    inner: Weak<Inner>,
    listener: Weak<Listener>,
    runtime: tokio::runtime::Handle,
) {
    loop {
        let Ok((stream, _)) = socket.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let (Some(strong_inner), Some(strong_listener)) = (inner.upgrade(), listener.upgrade())
        else {
            return;
        };
        if strong_listener.open_sockets.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
            strong_listener.open_sockets.fetch_sub(1, Ordering::Relaxed);
            drop(stream);
            continue;
        }
        stream.set_nodelay(true).ok();
        let task = strong_inner.next_id();
        let (abort, registration) = AbortHandle::new_pair();
        strong_listener
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(task, abort);
        let guard = TaskGuard {
            inner: inner.clone(),
            listener: listener.clone(),
            task,
            connection: None,
        };
        let protocol = strong_listener.spec.protocol;
        let work = async move {
            match protocol {
                ListenerProtocol::McpWebsocket | ListenerProtocol::Websocket => {
                    serve_websocket(strong_inner, strong_listener, stream, guard).await
                }
                ListenerProtocol::McpStreamableHttp | ListenerProtocol::Http => {
                    serve_http(strong_inner, strong_listener, stream, guard).await
                }
            }
        };
        runtime.spawn(Abortable::new(work, registration));
    }
}

async fn serve_websocket(
    inner: Arc<Inner>,
    listener: Arc<Listener>,
    stream: TcpStream,
    mut guard: TaskGuard,
) {
    let spec = listener.spec.clone();
    let token = listener.token.clone();
    let port = listener.port;
    // The error type is tungstenite's handshake API.
    #[allow(clippy::result_large_err)]
    let callback = move |request: &Request, mut response: Response| {
        let refuse = |status: u16| -> ErrorResponse {
            let mut response = http::Response::new(None);
            *response.status_mut() =
                http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::FORBIDDEN);
            response
        };
        let header = |name: &str| {
            request
                .headers()
                .get_all(name)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .collect::<Vec<_>>()
        };
        let target = request
            .uri()
            .path_and_query()
            .map_or("/", |target| target.as_str());
        admit(&spec, &token, port, target, &header).map_err(|Refused(status)| refuse(status))?;
        let offers_mcp = header("sec-websocket-protocol").iter().any(|offered| {
            offered
                .split(',')
                .any(|protocol| protocol.trim() == MCP_SUBPROTOCOL)
        });
        if spec.protocol == ListenerProtocol::McpWebsocket && offers_mcp {
            response.headers_mut().insert(
                "sec-websocket-protocol",
                http::HeaderValue::from_static(MCP_SUBPROTOCOL),
            );
        }
        Ok(response)
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let handshake =
        async_tungstenite::tokio::accept_hdr_async_with_config(stream, callback, Some(config));
    let Ok(Ok(socket)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await else {
        return;
    };
    let connection = inner.next_id();
    let (outbound, mut queued) = mpsc::unbounded::<String>();
    {
        let mut connections = listener
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if connections.len() >= MAX_CONNECTIONS {
            return;
        }
        connections.insert(
            connection,
            Connection {
                outbound: Some((connection, outbound.clone())),
                session: None,
            },
        );
    }
    guard.connection = Some(connection);
    let (mut writer, reader) = socket.split();
    let mut reader = reader.fuse();
    let mut rate = RateLimit::new();
    let mcp = listener.spec.protocol == ListenerProtocol::McpWebsocket;
    loop {
        futures::select_biased! {
            text = queued.next() => {
                let Some(text) = text else { break };
                if writer.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
            message = reader.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                    _ => break,
                };
                if !rate.allow() {
                    break;
                }
                if !mcp {
                    (inner.sink)(ListenerEvent::Message {
                        listener: listener.spec.name.clone(),
                        connection,
                        text: text.as_str().to_owned(),
                    });
                    continue;
                }
                let handled = match serde_json::from_str::<Value>(text.as_str()) {
                    Ok(message) => inner.handle_mcp(&listener, connection, message),
                    Err(_) => Handled::Reply(rpc_error(&Value::Null, PARSE_ERROR, "parse error")),
                };
                match handled {
                    Handled::Reply(reply) => {
                        outbound.unbounded_send(reply.to_string()).ok();
                    }
                    Handled::Accepted => {}
                    Handled::Call { request_id, receiver } => {
                        let outbound = outbound.clone();
                        tokio::spawn(async move {
                            let answer = receiver.await;
                            // A call cancelled by its connection or the client gets no answer.
                            if answer.is_ok() {
                                let reply = call_answer(&request_id, answer);
                                outbound.unbounded_send(reply.to_string()).ok();
                            }
                        });
                    }
                }
            }
        }
    }
    writer.close(None).await.ok();
}

/// One parsed HTTP/1.1 request.
struct HttpRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(path, _)| path)
    }
}

/// A keep-alive HTTP/1.1 connection: requests in order, with what was read ahead kept.
struct HttpConnection {
    stream: TcpStream,
    buffer: Vec<u8>,
}

enum ReadError {
    Closed,
    Refused(u16),
}

impl HttpConnection {
    async fn fill(&mut self) -> Result<(), ReadError> {
        let mut chunk = [0u8; 8192];
        match self.stream.read(&mut chunk).await {
            Ok(0) | Err(_) => Err(ReadError::Closed),
            Ok(read) => {
                if self.buffer.len() + read > MAX_HEAD_BYTES + MAX_MESSAGE_BYTES {
                    return Err(ReadError::Refused(413));
                }
                self.buffer.extend_from_slice(&chunk[..read]);
                Ok(())
            }
        }
    }

    async fn read_request(&mut self) -> Result<HttpRequest, ReadError> {
        let head_end = loop {
            if let Some(end) = self
                .buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
            {
                break end;
            }
            if self.buffer.len() > MAX_HEAD_BYTES {
                return Err(ReadError::Refused(431));
            }
            self.fill().await?;
        };
        let head = std::str::from_utf8(&self.buffer[..head_end])
            .map_err(|_| ReadError::Refused(400))?
            .to_owned();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default();
        let mut parts = request_line.split(' ');
        let (Some(method), Some(target), Some(version), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ReadError::Refused(400));
        };
        if version != "HTTP/1.1" && version != "HTTP/1.0" {
            return Err(ReadError::Refused(505));
        }
        let mut headers = Vec::new();
        for line in lines {
            if line.starts_with([' ', '\t']) {
                return Err(ReadError::Refused(400));
            }
            let (name, value) = line.split_once(':').ok_or(ReadError::Refused(400))?;
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
        let mut request = HttpRequest {
            method: method.to_owned(),
            target: target.to_owned(),
            headers,
            body: Vec::new(),
        };
        if !request.header("transfer-encoding").is_empty() {
            return Err(ReadError::Refused(411));
        }
        let length = match request.header("content-length").as_slice() {
            [] => 0,
            [length] => length
                .parse::<usize>()
                .map_err(|_| ReadError::Refused(400))?,
            _ => return Err(ReadError::Refused(400)),
        };
        if length > MAX_MESSAGE_BYTES {
            return Err(ReadError::Refused(413));
        }
        let body_start = head_end + 4;
        while self.buffer.len() < body_start + length {
            self.fill().await?;
        }
        request.body = self.buffer[body_start..body_start + length].to_vec();
        self.buffer.drain(..body_start + length);
        Ok(request)
    }

    async fn write(
        &mut self,
        status: u16,
        headers: &[(&str, &str)],
        body: &[u8],
        close: bool,
    ) -> std::io::Result<()> {
        let reason = http::StatusCode::from_u16(status)
            .ok()
            .and_then(|status| status.canonical_reason())
            .unwrap_or("");
        use std::fmt::Write as _;
        let mut head = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            write!(head, "{name}: {value}\r\n").ok();
        }
        if close {
            head.push_str("connection: close\r\n");
        }
        head.push_str("\r\n");
        self.stream.write_all(head.as_bytes()).await?;
        self.stream.write_all(body).await?;
        self.stream.flush().await
    }

    /// Waits for `receiver` while watching the socket: `None` when the client went away. Bytes
    /// the client sends meanwhile are kept for the next request.
    async fn wait<T>(
        &mut self,
        receiver: oneshot::Receiver<T>,
    ) -> Option<Result<T, oneshot::Canceled>> {
        let mut receiver = receiver.fuse();
        loop {
            let fill = self.fill().fuse();
            futures::pin_mut!(fill);
            futures::select_biased! {
                answer = receiver => return Some(answer),
                filled = fill => {
                    if filled.is_err() {
                        return None;
                    }
                }
            }
        }
    }
}

async fn serve_http(
    inner: Arc<Inner>,
    listener: Arc<Listener>,
    stream: TcpStream,
    _guard: TaskGuard,
) {
    let mut connection = HttpConnection {
        stream,
        buffer: Vec::new(),
    };
    let mut rate = RateLimit::new();
    loop {
        let request = match connection.read_request().await {
            Ok(request) => request,
            Err(ReadError::Closed) => return,
            Err(ReadError::Refused(status)) => {
                connection.write(status, &[], b"", true).await.ok();
                return;
            }
        };
        let header = |name: &str| request.header(name);
        if let Err(Refused(status)) = admit(
            &listener.spec,
            &listener.token,
            listener.port,
            &request.target,
            &header,
        ) {
            connection.write(status, &[], b"", true).await.ok();
            return;
        }
        if !rate.allow() {
            connection.write(429, &[], b"", true).await.ok();
            return;
        }
        let keep_going = match listener.spec.protocol {
            ListenerProtocol::Http => {
                serve_raw_request(&inner, &listener, &mut connection, request).await
            }
            _ => serve_mcp_request(&inner, &listener, &mut connection, request).await,
        };
        if !keep_going {
            return;
        }
    }
}

/// An `http` listener's request: to the extension, and its `reply` back.
async fn serve_raw_request(
    inner: &Inner,
    listener: &Listener,
    connection: &mut HttpConnection,
    request: HttpRequest,
) -> bool {
    let Ok(body) = String::from_utf8(request.body.clone()) else {
        return connection.write(400, &[], b"", true).await.is_ok();
    };
    let id = inner.next_id();
    let (sender, receiver) = oneshot::channel();
    inner
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, sender);
    (inner.sink)(ListenerEvent::Request {
        id,
        listener: listener.spec.name.clone(),
        path: request.path().to_owned(),
        body,
    });
    match connection.wait(receiver).await {
        Some(Ok((status, body))) => connection
            .write(
                status,
                &[("content-type", "application/json")],
                body.as_bytes(),
                false,
            )
            .await
            .is_ok(),
        Some(Err(oneshot::Canceled)) => connection.write(503, &[], b"", true).await.is_ok(),
        None => {
            inner
                .requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            false
        }
    }
}

/// The session a request names, by its `Mcp-Session-Id`.
fn session_connection(listener: &Listener, request: &HttpRequest) -> Option<u64> {
    let session = *request.header(SESSION_HEADER).first()?;
    listener
        .connections
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(_, connection)| connection.session.as_deref() == Some(session))
        .map(|(id, _)| *id)
}

/// A new session for `initialize`. At the limit, the oldest session without an event stream
/// makes room; with none, there is no room.
fn new_session(inner: &Inner, listener: &Listener) -> Option<(u64, String)> {
    let session = new_token().ok()?;
    let id = inner.next_id();
    let evicted = {
        let mut connections = listener
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut evicted = None;
        if connections.len() >= MAX_CONNECTIONS {
            let oldest = connections
                .iter()
                .filter(|(_, connection)| connection.outbound.is_none())
                .map(|(id, _)| *id)
                .min()?;
            connections.remove(&oldest);
            evicted = Some(oldest);
        }
        connections.insert(
            id,
            Connection {
                outbound: None,
                session: Some(session.clone()),
            },
        );
        evicted
    };
    if let Some(evicted) = evicted {
        inner.end_connection(listener, evicted);
    }
    Some((id, session))
}

/// An `mcp-streamable-http` request on `/mcp`: `POST` a message, `GET` the event stream,
/// `DELETE` the session.
async fn serve_mcp_request(
    inner: &Inner,
    listener: &Listener,
    connection: &mut HttpConnection,
    request: HttpRequest,
) -> bool {
    if request.path() != MCP_PATH {
        return connection.write(404, &[], b"", false).await.is_ok();
    }
    let json_type = ("content-type", "application/json");
    match request.method.as_str() {
        "POST" => {
            let message = match serde_json::from_slice::<Value>(&request.body) {
                Ok(message) => message,
                Err(_) => {
                    let error = rpc_error(&Value::Null, PARSE_ERROR, "parse error").to_string();
                    return connection
                        .write(400, &[json_type], error.as_bytes(), false)
                        .await
                        .is_ok();
                }
            };
            let initialize = message.get("method").and_then(Value::as_str) == Some("initialize");
            let (session_id, new_session_header) = if initialize {
                match new_session(inner, listener) {
                    Some((id, session)) => (id, Some(session)),
                    None => return connection.write(503, &[], b"", false).await.is_ok(),
                }
            } else if request.header(SESSION_HEADER).is_empty() {
                return connection.write(400, &[], b"", false).await.is_ok();
            } else {
                match session_connection(listener, &request) {
                    Some(id) => (id, None),
                    None => return connection.write(404, &[], b"", false).await.is_ok(),
                }
            };
            let mut headers = vec![json_type];
            if let Some(session) = &new_session_header {
                headers.push((SESSION_HEADER, session.as_str()));
            }
            match inner.handle_mcp(listener, session_id, message) {
                Handled::Reply(reply) => connection
                    .write(200, &headers, reply.to_string().as_bytes(), false)
                    .await
                    .is_ok(),
                Handled::Accepted => connection.write(202, &[], b"", false).await.is_ok(),
                Handled::Call {
                    request_id,
                    receiver,
                } => match connection.wait(receiver).await {
                    Some(answer) => {
                        let reply = call_answer(&request_id, answer).to_string();
                        connection
                            .write(200, &headers, reply.as_bytes(), false)
                            .await
                            .is_ok()
                    }
                    None => {
                        let name = listener.spec.name.as_str();
                        inner.cancel_calls(|call| {
                            call.listener == name
                                && call.connection == session_id
                                && call.request_id == request_id
                        });
                        false
                    }
                },
            }
        }
        "GET" => {
            let accepts_events = request
                .header("accept")
                .iter()
                .any(|accept| accept.contains("text/event-stream"));
            if !accepts_events {
                return connection.write(406, &[], b"", false).await.is_ok();
            }
            let Some(session_id) = session_connection(listener, &request) else {
                return connection.write(404, &[], b"", false).await.is_ok();
            };
            serve_event_stream(inner, listener, connection, session_id).await;
            false
        }
        "DELETE" => {
            let Some(session_id) = session_connection(listener, &request) else {
                return connection.write(404, &[], b"", false).await.is_ok();
            };
            inner.end_connection(listener, session_id);
            connection.write(200, &[], b"", false).await.is_ok()
        }
        _ => connection
            .write(405, &[("allow", "GET, POST, DELETE")], b"", false)
            .await
            .is_ok(),
    }
}

/// A session's server-sent events: notifications, until the client goes away, the session ends
/// or a newer stream replaces this one.
async fn serve_event_stream(
    inner: &Inner,
    listener: &Listener,
    connection: &mut HttpConnection,
    session_id: u64,
) {
    let stream_id = inner.next_id();
    let (sender, receiver) = mpsc::unbounded::<String>();
    {
        let mut connections = listener
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(session) = connections.get_mut(&session_id) else {
            return;
        };
        session.outbound = Some((stream_id, sender));
    }
    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\n\
                transfer-encoding: chunked\r\n\r\n";
    if connection.stream.write_all(head.as_bytes()).await.is_ok() {
        let mut receiver = receiver.fuse();
        loop {
            let event = {
                let fill = connection.fill().fuse();
                futures::pin_mut!(fill);
                futures::select_biased! {
                    event = receiver.next() => event,
                    filled = fill => match filled {
                        Ok(()) => continue,
                        Err(_) => None,
                    },
                }
            };
            let Some(event) = event else {
                break;
            };
            let data = format!("event: message\ndata: {event}\n\n");
            let chunk = format!("{:x}\r\n{data}\r\n", data.len());
            let stream = &mut connection.stream;
            if stream.write_all(chunk.as_bytes()).await.is_err() || stream.flush().await.is_err() {
                break;
            }
        }
    }
    let mut connections = listener
        .connections
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(session) = connections.get_mut(&session_id)
        && session
            .outbound
            .as_ref()
            .is_some_and(|(id, _)| *id == stream_id)
    {
        session.outbound = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_tungstenite::tungstenite::client::IntoClientRequest as _;
    use ui_ext::integration::ListenerAuth;

    fn spec(protocol: ListenerProtocol, auth: ListenerAuth) -> ListenerSpec {
        ListenerSpec {
            name: "ide".into(),
            protocol,
            auth,
            token_in_env: false,
            reason: "IDE".into(),
        }
    }

    fn header_auth() -> ListenerAuth {
        ListenerAuth {
            header: Some("X-Claude-Code-Ide-Authorization".into()),
            ..Default::default()
        }
    }

    fn bearer_auth() -> ListenerAuth {
        ListenerAuth {
            bearer: true,
            ..Default::default()
        }
    }

    #[derive(Default)]
    struct Observer(Mutex<Vec<String>>);

    impl ListenerObserver for Observer {
        fn opened(&self, extension_id: &str, listener: &OpenListener) {
            self.0.lock().unwrap().push(format!(
                "opened {extension_id} {} {}",
                listener.name,
                listener.discovery_files.len()
            ));
        }
        fn closed(&self, extension_id: &str, listener: &str) {
            self.0
                .lock()
                .unwrap()
                .push(format!("closed {extension_id} {listener}"));
        }
    }

    struct Setup {
        set: ListenerSet,
        events: mpsc::UnboundedReceiver<ListenerEvent>,
        observer: Arc<Observer>,
        port: u16,
        token: String,
    }

    fn setup(spec: &ListenerSpec) -> Setup {
        let (sender, events) = mpsc::unbounded();
        let observer = Arc::new(Observer::default());
        let set = ListenerSet::new(
            "test-ext".into(),
            Arc::new(move |event| {
                sender.unbounded_send(event).ok();
            }),
            Some(observer.clone()),
        );
        let port = set
            .open(&tokio::runtime::Handle::current(), spec, Vec::new())
            .unwrap();
        let token = set.token(&spec.name).unwrap();
        assert_eq!(set.port(&spec.name), Some(port));
        Setup {
            set,
            events,
            observer,
            port,
            token,
        }
    }

    fn tool() -> ToolDef {
        ToolDef {
            name: "openDiff".into(),
            description: "Opens a diff".into(),
            input_schema: r#"{"type":"object"}"#.into(),
        }
    }

    #[test]
    fn admission() {
        let spec = spec(ListenerProtocol::McpWebsocket, header_auth());
        let check = |headers: &[(&str, &str)], target: &str| {
            let header = |name: &str| -> Vec<&str> {
                headers
                    .iter()
                    .filter(|(key, _)| *key == name)
                    .map(|(_, value)| *value)
                    .collect()
            };
            admit(&spec, "secret", 4000, target, &header)
        };
        let token = ("x-claude-code-ide-authorization", "secret");
        assert_eq!(check(&[("host", "127.0.0.1:4000"), token], "/"), Ok(()));
        assert_eq!(check(&[("host", "LOCALHOST:4000"), token], "/"), Ok(()));
        assert_eq!(check(&[("host", "127.0.0.1:4000")], "/"), Err(Refused(401)));
        assert_eq!(
            check(
                &[
                    ("host", "127.0.0.1:4000"),
                    ("x-claude-code-ide-authorization", "secreT")
                ],
                "/"
            ),
            Err(Refused(401))
        );
        assert_eq!(
            check(
                &[
                    ("host", "127.0.0.1:4000"),
                    token,
                    ("origin", "https://evil.example")
                ],
                "/"
            ),
            Err(Refused(403))
        );
        assert_eq!(
            check(&[("host", "evil.example:4000"), token], "/"),
            Err(Refused(403))
        );
        assert_eq!(
            check(&[("host", "127.0.0.1:4001"), token], "/"),
            Err(Refused(403))
        );
        assert_eq!(check(&[token], "/"), Err(Refused(403)));
        assert_eq!(
            check(
                &[("host", "127.0.0.1:4000"), ("host", "evil.example"), token],
                "/"
            ),
            Err(Refused(400))
        );

        let bearer = super::tests::spec(ListenerProtocol::McpStreamableHttp, bearer_auth());
        let header = |name: &str| -> Vec<&str> {
            match name {
                "host" => vec!["localhost:4000"],
                "authorization" => vec!["Bearer secret"],
                _ => vec![],
            }
        };
        assert_eq!(admit(&bearer, "secret", 4000, "/mcp", &header), Ok(()));
        assert_eq!(
            admit(&bearer, "other", 4000, "/mcp", &header),
            Err(Refused(401))
        );

        let query = super::tests::spec(
            ListenerProtocol::Websocket,
            ListenerAuth {
                query: Some("auth".into()),
                ..Default::default()
            },
        );
        let header = |name: &str| -> Vec<&str> {
            if name == "host" {
                vec!["127.0.0.1:4000"]
            } else {
                vec![]
            }
        };
        assert_eq!(
            admit(&query, "secret", 4000, "/?a=1&auth=secret", &header),
            Ok(())
        );
        assert_eq!(
            admit(&query, "secret", 4000, "/?auth=no", &header),
            Err(Refused(401))
        );
        assert_eq!(
            admit(&query, "secret", 4000, "/?auth=secret&auth=x", &header),
            Err(Refused(400))
        );
    }

    #[test]
    fn tokens_are_random_and_long() {
        let a = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, new_token().unwrap());
    }

    async fn ws_connect(
        port: u16,
        headers: &[(&'static str, String)],
    ) -> Result<
        async_tungstenite::WebSocketStream<async_tungstenite::tokio::TokioAdapter<TcpStream>>,
        async_tungstenite::tungstenite::Error,
    > {
        let mut request = format!("ws://127.0.0.1:{port}/")
            .into_client_request()
            .unwrap();
        for (name, value) in headers {
            request
                .headers_mut()
                .insert(*name, http::HeaderValue::from_str(value).unwrap());
        }
        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        async_tungstenite::tokio::client_async(request, stream)
            .await
            .map(|(socket, response)| {
                if headers
                    .iter()
                    .any(|(name, _)| *name == "sec-websocket-protocol")
                {
                    assert_eq!(
                        response.headers().get("sec-websocket-protocol").unwrap(),
                        MCP_SUBPROTOCOL
                    );
                }
                socket
            })
    }

    async fn next_text(
        socket: &mut async_tungstenite::WebSocketStream<
            async_tungstenite::tokio::TokioAdapter<TcpStream>,
        >,
    ) -> Value {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
                Message::Close(_) => panic!("closed"),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn mcp_websocket_round_trip_and_cancellation() {
        let spec = spec(ListenerProtocol::McpWebsocket, header_auth());
        let mut setup = setup(&spec);
        setup.set.register_tools("ide", vec![tool()]).unwrap();

        // Refused before the extension sees anything.
        assert!(ws_connect(setup.port, &[]).await.is_err());
        let wrong = [("x-claude-code-ide-authorization", "nope".to_string())];
        assert!(ws_connect(setup.port, &wrong).await.is_err());
        let with_origin = [
            ("x-claude-code-ide-authorization", setup.token.clone()),
            ("origin", "https://evil.example".to_string()),
        ];
        assert!(ws_connect(setup.port, &with_origin).await.is_err());

        let headers = [
            ("x-claude-code-ide-authorization", setup.token.clone()),
            ("sec-websocket-protocol", MCP_SUBPROTOCOL.to_string()),
        ];
        let mut socket = ws_connect(setup.port, &headers).await.unwrap();
        let send = |value: Value| Message::text(value.to_string());
        socket
            .send(send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}})))
            .await
            .unwrap();
        let reply = next_text(&mut socket).await;
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["serverInfo"]["name"], "test-ext");
        socket
            .send(send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})))
            .await
            .unwrap();
        let reply = next_text(&mut socket).await;
        assert_eq!(reply["result"]["tools"][0]["name"], "openDiff");
        assert_eq!(reply["result"]["tools"][0]["inputSchema"]["type"], "object");

        socket
            .send(send(
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope"}}),
            ))
            .await
            .unwrap();
        assert_eq!(
            next_text(&mut socket).await["error"]["code"],
            INVALID_PARAMS
        );

        socket
            .send(send(json!({"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"openDiff","arguments":{"x":1}}})))
            .await
            .unwrap();
        let ListenerEvent::McpCall(call) = setup.events.next().await.unwrap() else {
            panic!("a call")
        };
        assert_eq!(call.tool, "openDiff");
        assert_eq!(call.arguments, r#"{"x":1}"#);
        assert!(setup.set.respond(call.call_id, Ok("[]".into())).is_err());
        setup
            .set
            .notify(
                "ide",
                Some(call.connection),
                "ide/diffAccepted",
                r#"{"filePath":"a"}"#,
            )
            .unwrap();
        assert_eq!(next_text(&mut socket).await["method"], "ide/diffAccepted");
        setup
            .set
            .respond(
                call.call_id,
                Ok(r#"{"content":[{"type":"text","text":"FILE_SAVED"}]}"#.into()),
            )
            .unwrap();
        let reply = next_text(&mut socket).await;
        assert_eq!(reply["id"], "a");
        assert_eq!(reply["result"]["content"][0]["text"], "FILE_SAVED");
        assert!(
            setup.set.respond(call.call_id, Ok("{}".into())).is_err(),
            "answered once"
        );

        // A blocking call is cancelled when its connection closes.
        socket
            .send(send(
                json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"openDiff"}}),
            ))
            .await
            .unwrap();
        let ListenerEvent::McpCall(call) = setup.events.next().await.unwrap() else {
            panic!("a call")
        };
        assert_eq!(call.arguments, "{}");
        drop(socket);
        assert_eq!(
            setup.events.next().await.unwrap(),
            ListenerEvent::McpCancelled(call.call_id)
        );
        assert!(setup.set.respond(call.call_id, Ok("{}".into())).is_err());

        // Closing (a revocation) ends the listener and tells the observer.
        setup.set.close("ide");
        assert!(!setup.set.is_open("ide"));
        assert_eq!(
            *setup.observer.0.lock().unwrap(),
            ["opened test-ext ide 0", "closed test-ext ide"]
        );
    }

    #[tokio::test]
    async fn raw_websocket_messages() {
        let spec = spec(
            ListenerProtocol::Websocket,
            ListenerAuth {
                query: Some("auth".into()),
                ..Default::default()
            },
        );
        let mut setup = setup(&spec);
        assert!(setup.set.register_tools("ide", vec![tool()]).is_err());
        let mut request = format!("ws://127.0.0.1:{}/?auth={}", setup.port, setup.token)
            .into_client_request()
            .unwrap();
        request.headers_mut().remove("origin");
        let stream = TcpStream::connect(("127.0.0.1", setup.port)).await.unwrap();
        let (mut socket, _) = async_tungstenite::tokio::client_async(request, stream)
            .await
            .unwrap();
        socket.send(Message::text("{\"hi\":1}")).await.unwrap();
        let ListenerEvent::Message {
            connection, text, ..
        } = setup.events.next().await.unwrap()
        else {
            panic!("a message")
        };
        assert_eq!(text, "{\"hi\":1}");
        setup.set.send("ide", connection, "back").unwrap();
        match socket.next().await.unwrap().unwrap() {
            Message::Text(text) => assert_eq!(text.as_str(), "back"),
            other => panic!("{other:?}"),
        }
        assert!(setup.set.send("ide", connection + 100, "x").is_err());
    }

    async fn http(port: u16, request: &str) -> (u16, String, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        read_response(&mut stream).await
    }

    async fn read_response(stream: &mut TcpStream) -> (u16, String, String) {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8(buffer[..end].to_vec()).unwrap();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .map_or(0, |length| length.parse::<usize>().unwrap());
                if buffer.len() >= end + 4 + length {
                    let status = head[9..12].parse().unwrap();
                    let body =
                        String::from_utf8(buffer[end + 4..end + 4 + length].to_vec()).unwrap();
                    return (status, head, body);
                }
            }
            let read = stream.read(&mut chunk).await.unwrap();
            assert!(
                read > 0,
                "closed early: {}",
                String::from_utf8_lossy(&buffer)
            );
            buffer.extend_from_slice(&chunk[..read]);
        }
    }

    fn post(port: u16, token: &str, session: Option<&str>, body: &Value) -> String {
        let body = body.to_string();
        let session = session.map_or(String::new(), |session| {
            format!("Mcp-Session-Id: {session}\r\n")
        });
        format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
             {session}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn mcp_streamable_http() {
        let spec = spec(ListenerProtocol::McpStreamableHttp, bearer_auth());
        let mut setup = setup(&spec);
        setup.set.register_tools("ide", vec![tool()]).unwrap();
        let port = setup.port;
        let token = setup.token.clone();
        let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}});

        // Refusals.
        let (status, _, _) = http(port, &post(port, "wrong", None, &initialize)).await;
        assert_eq!(status, 401);
        let foreign_host = post(port, &token, None, &initialize)
            .replace(&format!("Host: 127.0.0.1:{port}"), "Host: evil.example");
        assert_eq!(http(port, &foreign_host).await.0, 403);
        let with_origin = post(port, &token, None, &initialize).replace(
            "Content-Type",
            "Origin: http://evil.example\r\nContent-Type",
        );
        assert_eq!(http(port, &with_origin).await.0, 403);
        let other_path = post(port, &token, None, &initialize).replace("POST /mcp", "POST /other");
        assert_eq!(http(port, &other_path).await.0, 404);

        // Initialize opens a session; later requests need it.
        let (status, head, body) = http(port, &post(port, &token, None, &initialize)).await;
        assert_eq!(status, 200);
        let session = head
            .lines()
            .find_map(|line| line.strip_prefix("mcp-session-id: "))
            .unwrap()
            .to_owned();
        let reply: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        let list = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        assert_eq!(http(port, &post(port, &token, None, &list)).await.0, 400);
        assert_eq!(
            http(port, &post(port, &token, Some("nope"), &list)).await.0,
            404
        );
        let initialized = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert_eq!(
            http(port, &post(port, &token, Some(&session), &initialized))
                .await
                .0,
            202
        );

        // The event stream carries notifications.
        let mut events = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        events
            .write_all(
                format!(
                    "GET /mcp HTTP/1.1\r\nHost: localhost:{port}\r\nAuthorization: Bearer {token}\r\n\
                     Accept: text/event-stream\r\nMcp-Session-Id: {session}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut seen = Vec::new();
        let mut chunk = [0u8; 4096];
        while !String::from_utf8_lossy(&seen).contains("text/event-stream") {
            let read = events.read(&mut chunk).await.unwrap();
            seen.extend_from_slice(&chunk[..read]);
        }

        // A blocking call over keep-alive, answered while a notification goes out.
        let mut caller = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let call = json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"openDiff","arguments":{"a":"b"}}});
        caller
            .write_all(post(port, &token, Some(&session), &call).as_bytes())
            .await
            .unwrap();
        let ListenerEvent::McpCall(mcp_call) = setup.events.next().await.unwrap() else {
            panic!("a call")
        };
        setup
            .set
            .notify("ide", None, "ide/diffAccepted", r#"{"content":"x"}"#)
            .unwrap();
        while !String::from_utf8_lossy(&seen).contains("ide/diffAccepted") {
            let read = events.read(&mut chunk).await.unwrap();
            seen.extend_from_slice(&chunk[..read]);
        }
        setup
            .set
            .respond(
                mcp_call.call_id,
                Err(McpError {
                    code: -32000,
                    message: "no".into(),
                }),
            )
            .unwrap();
        let (status, _, body) = read_response(&mut caller).await;
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["code"], -32000);

        // The same connection again; then a call whose client goes away is cancelled.
        caller
            .write_all(post(port, &token, Some(&session), &call).as_bytes())
            .await
            .unwrap();
        let ListenerEvent::McpCall(mcp_call) = setup.events.next().await.unwrap() else {
            panic!("a call")
        };
        drop(caller);
        assert_eq!(
            setup.events.next().await.unwrap(),
            ListenerEvent::McpCancelled(mcp_call.call_id)
        );

        // DELETE ends the session.
        let delete = format!(
            "DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
             Mcp-Session-Id: {session}\r\n\r\n"
        );
        assert_eq!(http(port, &delete).await.0, 200);
        assert_eq!(
            http(port, &post(port, &token, Some(&session), &list))
                .await
                .0,
            404
        );
    }

    #[tokio::test]
    async fn raw_http_requests() {
        let spec = spec(ListenerProtocol::Http, bearer_auth());
        let mut setup = setup(&spec);
        let port = setup.port;
        let request = format!(
            "POST /hook?x=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {}\r\n\
             Content-Length: 2\r\n\r\n{{}}",
            setup.token
        );
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let ListenerEvent::Request { id, path, body, .. } = setup.events.next().await.unwrap()
        else {
            panic!("a request")
        };
        assert_eq!((path.as_str(), body.as_str()), ("/hook", "{}"));
        assert!(setup.set.reply(id, 99, String::new()).is_err());
        setup.set.reply(id, 201, "{\"ok\":true}".into()).unwrap();
        let (status, _, body) = read_response(&mut stream).await;
        assert_eq!((status, body.as_str()), (201, "{\"ok\":true}"));
        assert!(setup.set.reply(id, 200, String::new()).is_err());
    }

    #[tokio::test]
    async fn tool_registration_rules() {
        let setup = setup(&spec(ListenerProtocol::McpWebsocket, header_auth()));
        let set = &setup.set;
        let named = |name: &str| ToolDef {
            name: name.into(),
            ..tool()
        };
        assert!(set.register_tools("ide", vec![named("bad name")]).is_err());
        assert!(
            set.register_tools("ide", vec![named("a"), named("a")])
                .is_err()
        );
        assert!(
            set.register_tools(
                "ide",
                vec![ToolDef {
                    input_schema: "[]".into(),
                    ..tool()
                }]
            )
            .is_err()
        );
        assert!(
            set.register_tools(
                "ide",
                (0..=MAX_TOOLS).map(|i| named(&format!("t{i}"))).collect()
            )
            .is_err()
        );
        assert!(set.register_tools("other", vec![tool()]).is_err());
        set.register_tools("ide", vec![tool()]).unwrap();
        assert!(set.notify("ide", None, "", "{}").is_err());
        assert!(set.notify("ide", None, "m", "1").is_err());
        assert!(set.notify("ide", Some(12345), "m", "{}").is_err());
        set.notify("ide", None, "m", "{}").unwrap();
    }
}
