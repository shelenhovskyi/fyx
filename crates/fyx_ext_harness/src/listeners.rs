//! Fake listeners for the `fyx:ui` 0.4 `network:listen` (design.md B.7), with the host's
//! checks (`ui_ext_wasm::listeners`, plan 5.11), so an extension's MCP or websocket server runs
//! headless.
//!
//! A declared `[[fyx_ui.listeners]]` entry is open while it is granted: the harness grants
//! every declared listener, as consenting at install does, and
//! [`crate::Harness::set_listener_granted`] revokes one (its connections close and their open
//! calls are cancelled). No socket is bound; a test plays the client:
//!
//! - [`crate::Harness::connect`] opens a connection to a websocket kind;
//!   [`crate::Harness::client_send`] sends it one text frame; [`crate::Harness::client_received`]
//!   takes what the client got since, as JSON text.
//! - On the `mcp-*` kinds the fake speaks MCP as the host does: it answers `initialize`
//!   ([`PROTOCOL_VERSIONS`]), `ping` and `tools/list` from the registered tools, and turns a
//!   `tools/call` of a registered tool into `io-event::mcp-call`, whose `respond` becomes the
//!   JSON-RPC answer. `notifications/initialized` after `initialize` is one
//!   `io-event::mcp-initialized` per connection, with the `clientInfo` cut as the host cuts it. [`crate::Harness::disconnect`] cancels the connection's open calls.
//! - On a `websocket` listener each frame is `io-event::listener-message`; on an `http` listener
//!   [`crate::Harness::http_request`] is `io-event::listener-request`, and
//!   [`crate::Harness::http_reply`] reads the `reply`.
//!
//! [`crate::Harness::deliver_io`] hands the events to the extension.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::{Value, json};
use ui_ext::integration::MAX_MCP_CLIENT_INFO_BYTES;
pub use ui_ext::integration::{ListenerEvent, MAX_TOOLS, McpCall, McpError, McpSession, ToolDef};
use ui_ext::integration::{ListenerProtocol, ListenerSpec, is_tool_name};
use ui_ext::v0_2::{HostError, HostErrorCode};

/// Open connections per listener.
pub const MAX_CONNECTIONS: usize = 16;
/// A message, a body, an answer or a tool's schema.
pub const MAX_MESSAGE_BYTES: usize = 4 << 20;
/// Supported MCP revisions, newest first; `initialize` echoes the client's when it is one.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_NOTIFICATION_METHOD_BYTES: usize = 128;
const MAX_TOOL_DESCRIPTION_BYTES: usize = 16 << 10;
/// The fake port of the first declared listener; the next ones count up.
const FIRST_PORT: u16 = 49152;

const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const PARSE_ERROR: i32 = -32700;

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

fn invalid(message: impl Into<String>) -> HostError {
    error(HostErrorCode::InvalidArgument, message)
}

fn is_mcp(protocol: ListenerProtocol) -> bool {
    matches!(
        protocol,
        ListenerProtocol::McpWebsocket | ListenerProtocol::McpStreamableHttp
    )
}

fn rpc_result(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn rpc_error(id: &Value, code: i32, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[derive(Debug)]
struct Connection {
    listener: String,
    /// What the client got and has not taken, in order.
    received: Vec<String>,
    /// `initialize`'s `clientInfo` (name, version), until `notifications/initialized` takes it.
    client: Option<(String, String)>,
}

#[derive(Debug)]
struct PendingCall {
    listener: String,
    connection: u64,
    request_id: Value,
}

/// The extension's listeners.
#[derive(Debug, Default)]
pub(crate) struct FakeListeners {
    /// `initialize`'s `serverInfo.name`.
    extension_id: String,
    specs: Vec<ListenerSpec>,
    revoked: BTreeSet<String>,
    tools: BTreeMap<String, Vec<ToolDef>>,
    connections: BTreeMap<u64, Connection>,
    calls: BTreeMap<u64, PendingCall>,
    /// Open `http` requests, by id: their listener.
    requests: BTreeMap<u64, String>,
    replies: BTreeMap<u64, (u16, String)>,
    next_id: u64,
    pub(crate) events: VecDeque<ListenerEvent>,
}

impl FakeListeners {
    pub(crate) fn set_specs(&mut self, specs: Vec<ListenerSpec>) {
        self.extension_id = "extension".into();
        self.specs = specs;
    }

    /// `initialize`'s `serverInfo.name`: the manifest's `id`.
    pub(crate) fn set_extension_id(&mut self, id: &str) {
        self.extension_id = id.to_owned();
    }

    pub(crate) fn specs(&self) -> &[ListenerSpec] {
        &self.specs
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// The declared listener `name`, if it is granted, as the host's import checks it.
    fn spec(&self, name: &str) -> Result<&ListenerSpec, HostError> {
        let spec = self
            .specs
            .iter()
            .find(|spec| spec.name == name)
            .ok_or_else(|| {
                error(
                    HostErrorCode::NotPermitted,
                    format!("no listener {name:?} is declared"),
                )
            })?;
        if self.revoked.contains(name) {
            return Err(error(
                HostErrorCode::Denied,
                format!("listener {name:?} is not granted"),
            ));
        }
        Ok(spec)
    }

    pub(crate) fn is_open(&self, name: &str) -> bool {
        self.spec(name).is_ok()
    }

    /// `${listen.<name>.port}`: a fake port while the listener is open.
    pub(crate) fn port(&self, name: &str) -> Option<u16> {
        let index = self.specs.iter().position(|spec| spec.name == name)?;
        self.is_open(name).then(|| FIRST_PORT + index as u16)
    }

    pub(crate) fn tools(&self, name: &str) -> &[ToolDef] {
        self.tools.get(name).map_or(&[], Vec::as_slice)
    }

    /// A grant or a revocation; a revoked listener closes.
    pub(crate) fn set_granted(&mut self, name: &str, granted: bool) {
        if granted {
            self.revoked.remove(name);
            return;
        }
        self.revoked.insert(name.to_owned());
        let closed: Vec<u64> = self
            .connections
            .iter()
            .filter(|(_, connection)| connection.listener == name)
            .map(|(id, _)| *id)
            .collect();
        for id in closed {
            self.disconnect(id);
        }
        self.requests.retain(|_, listener| listener != name);
    }

    fn send_to(
        &mut self,
        listener: &str,
        connection: Option<u64>,
        text: String,
    ) -> Result<(), HostError> {
        match connection {
            Some(id) => {
                let connection = self
                    .connections
                    .get_mut(&id)
                    .filter(|connection| connection.listener == listener)
                    .ok_or_else(|| invalid(format!("no open connection {id} on {listener:?}")))?;
                connection.received.push(text);
            }
            None => {
                for connection in self.connections.values_mut() {
                    if connection.listener == listener {
                        connection.received.push(text.clone());
                    }
                }
            }
        }
        Ok(())
    }

    // ---- The extension's side: the `listeners` imports. ----

    pub(crate) fn register_tools(
        &mut self,
        name: &str,
        tools: Vec<ToolDef>,
    ) -> Result<(), HostError> {
        let spec = self.spec(name)?;
        if !is_mcp(spec.protocol) {
            return Err(invalid(format!("listener {name:?} does not speak MCP")));
        }
        if tools.len() > MAX_TOOLS {
            return Err(error(
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
                return Err(error(
                    HostErrorCode::LimitExceeded,
                    format!("tool {:?} is too large", tool.name),
                ));
            }
            if !matches!(
                serde_json::from_str::<Value>(&tool.input_schema),
                Ok(Value::Object(_))
            ) {
                return Err(invalid(format!(
                    "tool {:?}: the input schema must be a JSON object",
                    tool.name
                )));
            }
        }
        let changed = self
            .tools
            .get(name)
            .map_or(!tools.is_empty(), |old| *old != tools);
        self.tools.insert(name.to_owned(), tools);
        if changed {
            let notification =
                json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" });
            self.send_to(name, None, notification.to_string())?;
        }
        Ok(())
    }

    pub(crate) fn respond(
        &mut self,
        call_id: u64,
        outcome: Result<String, McpError>,
    ) -> Result<(), HostError> {
        let outcome = match outcome {
            Ok(text) => {
                if text.len() > MAX_MESSAGE_BYTES {
                    return Err(error(
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
            .calls
            .remove(&call_id)
            .ok_or_else(|| invalid(format!("no open call {call_id}")))?;
        let answer = match outcome {
            Ok(result) => rpc_result(&call.request_id, result),
            Err(error) => rpc_error(&call.request_id, error.code, &error.message),
        };
        self.send_to(&call.listener, Some(call.connection), answer)
    }

    pub(crate) fn notify(
        &mut self,
        name: &str,
        connection: Option<u64>,
        method: &str,
        params: &str,
    ) -> Result<(), HostError> {
        if !is_mcp(self.spec(name)?.protocol) {
            return Err(invalid(format!("listener {name:?} does not speak MCP")));
        }
        if method.is_empty() || method.len() > MAX_NOTIFICATION_METHOD_BYTES {
            return Err(invalid("the method must be 1 to 128 bytes"));
        }
        if params.len() > MAX_MESSAGE_BYTES {
            return Err(error(
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
        self.send_to(name, connection, message.to_string())
    }

    pub(crate) fn send(
        &mut self,
        name: &str,
        connection: u64,
        text: &str,
    ) -> Result<(), HostError> {
        if self.spec(name)?.protocol != ListenerProtocol::Websocket {
            return Err(invalid(format!(
                "listener {name:?} is not a websocket listener"
            )));
        }
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("the text is more than {MAX_MESSAGE_BYTES} bytes"),
            ));
        }
        self.send_to(name, Some(connection), text.to_owned())
    }

    pub(crate) fn reply(
        &mut self,
        request_id: u64,
        status: u16,
        body: String,
    ) -> Result<(), HostError> {
        if !(200..=599).contains(&status) {
            return Err(invalid("the status must be 200 to 599"));
        }
        if body.len() > MAX_MESSAGE_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("the body is more than {MAX_MESSAGE_BYTES} bytes"),
            ));
        }
        self.requests
            .remove(&request_id)
            .ok_or_else(|| invalid(format!("no open request {request_id}")))?;
        self.replies.insert(request_id, (status, body));
        Ok(())
    }

    // ---- The client's side, for tests. ----

    pub(crate) fn connect(&mut self, name: &str) -> Result<u64, HostError> {
        let protocol = self.spec(name)?.protocol;
        if protocol == ListenerProtocol::Http {
            return Err(invalid(format!(
                "listener {name:?} is http: send requests with http_request"
            )));
        }
        let open = self
            .connections
            .values()
            .filter(|connection| connection.listener == name)
            .count();
        if open >= MAX_CONNECTIONS {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("listener {name:?} has {MAX_CONNECTIONS} connections"),
            ));
        }
        let id = self.next_id();
        self.connections.insert(
            id,
            Connection {
                listener: name.to_owned(),
                received: Vec::new(),
                client: None,
            },
        );
        Ok(id)
    }

    pub(crate) fn disconnect(&mut self, connection: u64) {
        if self.connections.remove(&connection).is_none() {
            return;
        }
        let cancelled: Vec<u64> = self
            .calls
            .iter()
            .filter(|(_, call)| call.connection == connection)
            .map(|(id, _)| *id)
            .collect();
        for id in cancelled {
            self.calls.remove(&id);
            self.events.push_back(ListenerEvent::McpCancelled(id));
        }
    }

    pub(crate) fn received(&mut self, connection: u64) -> Result<Vec<String>, HostError> {
        self.connections
            .get_mut(&connection)
            .map(|connection| std::mem::take(&mut connection.received))
            .ok_or_else(|| invalid(format!("no open connection {connection}")))
    }

    pub(crate) fn client_send(&mut self, connection: u64, text: &str) -> Result<(), HostError> {
        let listener = self
            .connections
            .get(&connection)
            .map(|connection| connection.listener.clone())
            .ok_or_else(|| invalid(format!("no open connection {connection}")))?;
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("the message is more than {MAX_MESSAGE_BYTES} bytes"),
            ));
        }
        let protocol = self.spec(&listener)?.protocol;
        if !is_mcp(protocol) {
            self.events.push_back(ListenerEvent::Message {
                listener,
                connection,
                text: text.to_owned(),
            });
            return Ok(());
        }
        let reply = match serde_json::from_str::<Value>(text) {
            Ok(message) => self.handle_mcp(&listener, connection, message),
            Err(_) => Some(rpc_error(&Value::Null, PARSE_ERROR, "not JSON")),
        };
        if let Some(reply) = reply {
            self.send_to(&listener, Some(connection), reply)?;
        }
        Ok(())
    }

    /// One JSON-RPC message of an `mcp-*` connection; the reply, if the host answers it.
    fn handle_mcp(&mut self, listener: &str, connection: u64, message: Value) -> Option<String> {
        let Value::Object(message) = message else {
            return Some(rpc_error(
                &Value::Null,
                INVALID_REQUEST,
                "batches and non-objects are not supported",
            ));
        };
        let id = message.get("id").cloned();
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id.map(|id| rpc_error(&id, INVALID_REQUEST, "not JSON-RPC 2.0"));
        }
        // Without a method: a response to a request the host never sends.
        let method = message.get("method").and_then(Value::as_str)?;
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            if method == "notifications/initialized"
                && let Some((client_name, client_version)) = self
                    .connections
                    .get_mut(&connection)
                    .and_then(|entry| entry.client.take())
            {
                self.events
                    .push_back(ListenerEvent::McpInitialized(McpSession {
                        listener: listener.to_owned(),
                        connection,
                        client_name,
                        client_version,
                    }));
            }
            if method == "notifications/cancelled"
                && let Some(request_id) = params.get("requestId")
            {
                let cancelled: Vec<u64> = self
                    .calls
                    .iter()
                    .filter(|(_, call)| {
                        call.connection == connection && &call.request_id == request_id
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for call in cancelled {
                    self.calls.remove(&call);
                    self.events.push_back(ListenerEvent::McpCancelled(call));
                }
            }
            return None;
        };
        match method {
            "initialize" => {
                let info = |key: &str| {
                    let text = params
                        .get("clientInfo")
                        .and_then(|info| info.get(key))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let mut end = text.len().min(MAX_MCP_CLIENT_INFO_BYTES);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text[..end].to_owned()
                };
                let client = (info("name"), info("version"));
                if let Some(entry) = self.connections.get_mut(&connection) {
                    entry.client = Some(client);
                }
                let requested = params.get("protocolVersion").and_then(Value::as_str);
                let version = requested
                    .filter(|requested| PROTOCOL_VERSIONS.contains(requested))
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                Some(rpc_result(
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": true } },
                        "serverInfo": {
                            "name": self.extension_id,
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                ))
            }
            "ping" => Some(rpc_result(&id, json!({}))),
            "tools/list" => {
                let tools: Vec<Value> = self
                    .tools(listener)
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "inputSchema": serde_json::from_str::<Value>(&tool.input_schema)
                                .unwrap_or_else(|_| json!({ "type": "object" })),
                        })
                    })
                    .collect();
                Some(rpc_result(&id, json!({ "tools": tools })))
            }
            "tools/call" => {
                let Some(tool) = params.get("name").and_then(Value::as_str) else {
                    return Some(rpc_error(&id, INVALID_PARAMS, "no tool name"));
                };
                if !self.tools(listener).iter().any(|known| known.name == tool) {
                    return Some(rpc_error(
                        &id,
                        INVALID_PARAMS,
                        &format!("unknown tool: {tool}"),
                    ));
                }
                let arguments = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(arguments @ Value::Object(_)) => arguments.clone(),
                    Some(_) => {
                        return Some(rpc_error(
                            &id,
                            INVALID_PARAMS,
                            "arguments must be an object",
                        ));
                    }
                };
                let call_id = self.next_id();
                self.calls.insert(
                    call_id,
                    PendingCall {
                        listener: listener.to_owned(),
                        connection,
                        request_id: id,
                    },
                );
                self.events.push_back(ListenerEvent::McpCall(McpCall {
                    call_id,
                    listener: listener.to_owned(),
                    connection,
                    tool: tool.to_owned(),
                    arguments: arguments.to_string(),
                }));
                None
            }
            _ => Some(rpc_error(
                &id,
                METHOD_NOT_FOUND,
                &format!("method not found: {method}"),
            )),
        }
    }

    pub(crate) fn http_request(
        &mut self,
        name: &str,
        path: &str,
        body: &str,
    ) -> Result<u64, HostError> {
        if self.spec(name)?.protocol != ListenerProtocol::Http {
            return Err(invalid(format!(
                "listener {name:?} is not an http listener"
            )));
        }
        let id = self.next_id();
        self.requests.insert(id, name.to_owned());
        self.events.push_back(ListenerEvent::Request {
            id,
            listener: name.to_owned(),
            path: path.to_owned(),
            body: body.to_owned(),
        });
        Ok(id)
    }

    pub(crate) fn take_reply(&mut self, request_id: u64) -> Option<(u16, String)> {
        self.replies.remove(&request_id)
    }

    /// The calls still waiting for `respond`.
    pub(crate) fn open_calls(&self) -> Vec<u64> {
        self.calls.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::integration::ListenerAuth;

    fn spec(name: &str, protocol: ListenerProtocol) -> ListenerSpec {
        ListenerSpec {
            name: name.into(),
            protocol,
            auth: ListenerAuth {
                header: Some("x-token".into()),
                ..ListenerAuth::default()
            },
            token_in_env: false,
            reason: "A test.".into(),
        }
    }

    fn listeners() -> FakeListeners {
        let mut listeners = FakeListeners::default();
        listeners.set_specs(vec![
            spec("ide", ListenerProtocol::McpWebsocket),
            spec("raw", ListenerProtocol::Websocket),
            spec("hooks", ListenerProtocol::Http),
        ]);
        listeners
    }

    fn echo() -> ToolDef {
        ToolDef {
            name: "echo".into(),
            description: "Echoes.".into(),
            input_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn received(listeners: &mut FakeListeners, connection: u64) -> Vec<Value> {
        listeners
            .received(connection)
            .unwrap()
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect()
    }

    #[test]
    fn mcp_calls_reach_the_extension_and_its_answers_the_client() {
        let mut listeners = listeners();
        let connection = listeners.connect("ide").unwrap();
        listeners.register_tools("ide", vec![echo()]).unwrap();
        assert_eq!(
            received(&mut listeners, connection),
            [json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })]
        );
        listeners
            .client_send(connection, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#)
            .unwrap();
        listeners
            .client_send(
                connection,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            )
            .unwrap();
        let replies = received(&mut listeners, connection);
        assert_eq!(
            replies[0]["result"]["protocolVersion"],
            PROTOCOL_VERSIONS[0]
        );
        assert_eq!(replies[1]["result"]["tools"][0]["name"], "echo");
        assert_eq!(
            replies[1]["result"]["tools"][0]["inputSchema"]["type"],
            "object"
        );

        listeners
            .client_send(
                connection,
                r#"{"jsonrpc":"2.0","id":"x","method":"tools/call","params":{"name":"echo"}}"#,
            )
            .unwrap();
        let Some(ListenerEvent::McpCall(call)) = listeners.events.pop_front() else {
            panic!("no call");
        };
        assert_eq!(
            (call.tool.as_str(), call.arguments.as_str()),
            ("echo", "{}")
        );
        assert!(received(&mut listeners, connection).is_empty());
        assert_eq!(
            listeners
                .respond(call.call_id, Ok("[]".into()))
                .unwrap_err()
                .code,
            HostErrorCode::InvalidArgument
        );
        listeners
            .respond(call.call_id, Ok(r#"{"content":[]}"#.into()))
            .unwrap();
        assert_eq!(
            received(&mut listeners, connection),
            [json!({ "jsonrpc": "2.0", "id": "x", "result": { "content": [] } })]
        );
        assert!(listeners.respond(call.call_id, Ok("{}".into())).is_err());

        listeners
            .notify("ide", None, "selection_changed", r#"{"a":1}"#)
            .unwrap();
        assert_eq!(
            received(&mut listeners, connection)[0]["method"],
            "selection_changed"
        );
        assert!(listeners.notify("ide", None, "x", "1").is_err());
        assert!(listeners.send("ide", connection, "x").is_err());
    }

    #[test]
    fn the_handshake_is_one_mcp_initialized_per_connection() {
        let mut listeners = listeners();
        let connection = listeners.connect("ide").unwrap();
        let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        listeners.client_send(connection, initialized).unwrap();
        assert!(listeners.events.is_empty(), "no initialize yet");
        let name = "é".repeat(200);
        let initialize = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "clientInfo": { "name": name, "version": "0.69.0" } } });
        listeners
            .client_send(connection, &initialize.to_string())
            .unwrap();
        listeners.client_send(connection, initialized).unwrap();
        listeners.client_send(connection, initialized).unwrap();
        let events: Vec<ListenerEvent> = listeners.events.drain(..).collect();
        let [ListenerEvent::McpInitialized(session)] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert_eq!(
            (session.listener.as_str(), session.connection),
            ("ide", connection)
        );
        assert_eq!(session.client_name.len(), MAX_MCP_CLIENT_INFO_BYTES);
        assert_eq!(session.client_version, "0.69.0");
    }

    #[test]
    fn closing_a_connection_cancels_its_calls() {
        let mut listeners = listeners();
        listeners.register_tools("ide", vec![echo()]).unwrap();
        let connection = listeners.connect("ide").unwrap();
        let call = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"echo"}}"#;
        listeners.client_send(connection, call).unwrap();
        listeners
            .client_send(connection, call.replace('5', "6").as_str())
            .unwrap();
        listeners
            .client_send(
                connection,
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":6}}"#,
            )
            .unwrap();
        listeners.disconnect(connection);
        let events: Vec<ListenerEvent> = listeners.events.drain(..).collect();
        let [
            ListenerEvent::McpCall(first),
            ListenerEvent::McpCall(second),
            cancelled_second,
            cancelled_first,
        ] = events.as_slice()
        else {
            panic!("{events:?}");
        };
        assert_eq!(
            *cancelled_second,
            ListenerEvent::McpCancelled(second.call_id)
        );
        assert_eq!(*cancelled_first, ListenerEvent::McpCancelled(first.call_id));
        assert!(listeners.open_calls().is_empty());
    }

    #[test]
    fn the_host_checks_hold() {
        let mut listeners = listeners();
        let code = |result: Result<(), HostError>| result.unwrap_err().code;
        assert_eq!(
            code(listeners.register_tools("none", vec![])),
            HostErrorCode::NotPermitted
        );
        assert_eq!(
            code(listeners.register_tools("raw", vec![echo()])),
            HostErrorCode::InvalidArgument
        );
        let mut bad = echo();
        bad.input_schema = "[]".into();
        assert_eq!(
            code(listeners.register_tools("ide", vec![bad])),
            HostErrorCode::InvalidArgument
        );
        assert_eq!(
            code(listeners.register_tools("ide", vec![echo(), echo()])),
            HostErrorCode::InvalidArgument
        );
        assert_eq!(
            code(listeners.register_tools("ide", vec![echo(); MAX_TOOLS + 1])),
            HostErrorCode::LimitExceeded
        );
        for _ in 0..MAX_CONNECTIONS {
            listeners.connect("raw").unwrap();
        }
        assert_eq!(
            listeners.connect("raw").unwrap_err().code,
            HostErrorCode::LimitExceeded
        );
        assert!(listeners.connect("hooks").is_err());

        listeners.set_granted("ide", false);
        assert_eq!(
            code(listeners.register_tools("ide", vec![echo()])),
            HostErrorCode::Denied
        );
        assert_eq!(listeners.port("ide"), None);
        listeners.set_granted("ide", true);
        assert_eq!(listeners.port("ide"), Some(FIRST_PORT));
    }

    #[test]
    fn raw_and_http_listeners_pass_text_through() {
        let mut listeners = listeners();
        let connection = listeners.connect("raw").unwrap();
        listeners.client_send(connection, "hello").unwrap();
        assert_eq!(
            listeners.events.pop_front(),
            Some(ListenerEvent::Message {
                listener: "raw".into(),
                connection,
                text: "hello".into(),
            })
        );
        listeners.send("raw", connection, "back").unwrap();
        assert_eq!(listeners.received(connection).unwrap(), ["back"]);

        let id = listeners.http_request("hooks", "/stop", "{}").unwrap();
        assert!(matches!(
            listeners.events.pop_front(),
            Some(ListenerEvent::Request { id: request, .. }) if request == id
        ));
        assert!(listeners.reply(id, 99, String::new()).is_err());
        listeners.reply(id, 200, "ok".into()).unwrap();
        assert_eq!(listeners.take_reply(id), Some((200, "ok".into())));
        assert!(listeners.reply(id, 200, String::new()).is_err());
        assert!(listeners.http_request("ide", "/", "").is_err());
    }
}
