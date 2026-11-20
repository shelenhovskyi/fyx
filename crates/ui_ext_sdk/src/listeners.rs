//! `fyx:ui` 0.4: loopback listeners and MCP (design.md B.7), with the `fyx-ui-0-4`
//! feature and the `network:listen` capability.
//!
//! The host binds every granted `[[fyx_ui.listeners]]` entry on `127.0.0.1:0` when the
//! extension activates, checks the client's token, `Origin` and `Host`, and for the `mcp-*`
//! kinds speaks MCP itself (`initialize`, `ping`, `tools/list`). The extension:
//!
//! - offers tools with [`register_tools`] (before the listener opens is fine: they apply when it
//!   does; registering again replaces them and tells open connections `tools/list_changed`);
//! - gets each `tools/call` of a registered tool in
//!   [`Extension::handle_io`](crate::Extension) as `IoEvent::McpCall` and answers it with
//!   [`respond`], now or later: a call stays open until answered, and `IoEvent::McpCancelled`
//!   says its connection closed or the client gave up;
//! - hears `IoEvent::McpInitialized` ([`McpSession`]: the connection and the client's
//!   `clientInfo`) once a client finishes the handshake, the moment to push its first context
//!   with [`notify`] to that connection;
//! - sends MCP notifications with [`notify`];
//! - on a raw `websocket` listener gets `IoEvent::ListenerMessage` and answers with [`send`]; on
//!   an `http` listener gets `IoEvent::ListenerRequest` and answers with [`reply`].
//!
//! The listener's port and token never reach the extension: `${listen.<name>.port}` and the
//! `[[fyx_ui.discovery_files]]` carry them to the client.
//!
//! As with the other host functions, a component imports only what it calls. Exported as 0.2 or
//! 0.1, every function here fails with `unavailable`. Off WASM each one records a
//! [`HostEffect::Listener`](crate::HostEffect::Listener) and succeeds.

use crate::HostError;
pub use crate::bindings::fyx::ui::listeners::{McpCall, McpError, McpSession, ToolDef};

/// JSON-RPC's "invalid params", for a call whose arguments the extension refuses.
pub const INVALID_PARAMS: i32 = -32602;
/// JSON-RPC's "internal error".
pub const INTERNAL_ERROR: i32 = -32603;

#[cfg(target_arch = "wasm32")]
unsafe extern "Rust" {
    safe fn __fyx_ui_listener_register_tools(
        listener: &str,
        tools: &[ToolDef],
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_listener_respond(
        call_id: u64,
        outcome: Result<&str, &McpError>,
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_listener_notify(
        listener: &str,
        connection: Option<u64>,
        method: &str,
        params: &str,
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_listener_send(
        listener: &str,
        connection: u64,
        text: &str,
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_listener_reply(
        request_id: u64,
        status: u16,
        body: &str,
    ) -> Result<(), HostError>;
}

/// What a listener function did, recorded off WASM in
/// [`HostEffect::Listener`](crate::HostEffect::Listener).
#[derive(Debug, Clone, PartialEq)]
pub enum ListenerEffect {
    RegisterTools {
        listener: String,
        tools: Vec<ToolDef>,
    },
    Respond {
        call_id: u64,
        outcome: Result<String, McpError>,
    },
    Notify {
        listener: String,
        connection: Option<u64>,
        method: String,
        params: String,
    },
    Send {
        listener: String,
        connection: u64,
        text: String,
    },
    Reply {
        request_id: u64,
        status: u16,
        body: String,
    },
}

#[cfg(not(target_arch = "wasm32"))]
fn record(effect: ListenerEffect) -> Result<(), HostError> {
    crate::host::record(crate::HostEffect::Listener(effect))
}

impl ToolDef {
    /// A tool: `name` is `[A-Za-z0-9_-]{1,64}`, `input_schema` a JSON schema object as text.
    /// MCP clients show the description to the model.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: input_schema.into(),
        }
    }
}

impl McpError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Offers `tools` on the `mcp-*` listener `listener`, replacing what it offered: at most 64,
/// uniquely named, each schema a JSON object.
pub fn register_tools(listener: &str, tools: &[ToolDef]) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_listener_register_tools(listener, tools);
    #[cfg(not(target_arch = "wasm32"))]
    record(ListenerEffect::RegisterTools {
        listener: listener.to_owned(),
        tools: tools.to_vec(),
    })
}

/// The answer to `IoEvent::McpCall` `call_id`: a whole MCP `CallToolResult` object as JSON text
/// (`{"content":[{"type":"text","text":"…"}]}`), or a JSON-RPC error. A call is answered once;
/// a cancelled or answered one is `invalid-argument`.
pub fn respond(call_id: u64, outcome: Result<&str, &McpError>) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_listener_respond(call_id, outcome);
    #[cfg(not(target_arch = "wasm32"))]
    record(ListenerEffect::Respond {
        call_id,
        outcome: outcome.map(str::to_owned).map_err(Clone::clone),
    })
}

/// An MCP notification on `listener`, to `connection` or to every open one; `params` is a JSON
/// object or array as text.
pub fn notify(
    listener: &str,
    connection: Option<u64>,
    method: &str,
    params: &str,
) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_listener_notify(listener, connection, method, params);
    #[cfg(not(target_arch = "wasm32"))]
    record(ListenerEffect::Notify {
        listener: listener.to_owned(),
        connection,
        method: method.to_owned(),
        params: params.to_owned(),
    })
}

/// A text frame on a `websocket` listener's connection.
pub fn send(listener: &str, connection: u64, text: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_listener_send(listener, connection, text);
    #[cfg(not(target_arch = "wasm32"))]
    record(ListenerEffect::Send {
        listener: listener.to_owned(),
        connection,
        text: text.to_owned(),
    })
}

/// The answer to an `http` listener's `IoEvent::ListenerRequest`: a status of 200 to 599 and a
/// body.
pub fn reply(request_id: u64, status: u16, body: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_listener_reply(request_id, status, body);
    #[cfg(not(target_arch = "wasm32"))]
    record(ListenerEffect::Reply {
        request_id,
        status,
        body: body.to_owned(),
    })
}

/// What `register_extension!`'s listener shims call, per exported world.
#[doc(hidden)]
pub mod glue {
    /// The 0.4 imports.
    pub mod newest {
        use super::super::{McpError, ToolDef};
        use crate::HostError;
        use crate::bindings::fyx::ui::listeners as wit;

        pub fn register_tools(listener: &str, tools: &[ToolDef]) -> Result<(), HostError> {
            wit::register_tools(listener, tools)
        }

        pub fn respond(call_id: u64, outcome: Result<&str, &McpError>) -> Result<(), HostError> {
            wit::respond(call_id, outcome)
        }

        pub fn notify(
            listener: &str,
            connection: Option<u64>,
            method: &str,
            params: &str,
        ) -> Result<(), HostError> {
            wit::notify(listener, connection, method, params)
        }

        pub fn send(listener: &str, connection: u64, text: &str) -> Result<(), HostError> {
            wit::send(listener, connection, text)
        }

        pub fn reply(request_id: u64, status: u16, body: &str) -> Result<(), HostError> {
            wit::reply(request_id, status, body)
        }
    }

    /// Worlds without listeners: every call is `unavailable`.
    macro_rules! older {
        ($world:ident, $version:literal) => {
            pub mod $world {
                use super::super::{McpError, ToolDef};
                use crate::{HostError, HostErrorCode};

                fn unavailable(what: &str) -> HostError {
                    HostError {
                        code: HostErrorCode::Unavailable,
                        message: format!(
                            "listeners::{what} needs fyx:ui 0.4; this extension is exported as {}",
                            $version
                        ),
                    }
                }

                pub fn register_tools(
                    _listener: &str,
                    _tools: &[ToolDef],
                ) -> Result<(), HostError> {
                    Err(unavailable("register_tools"))
                }

                pub fn respond(
                    _call_id: u64,
                    _outcome: Result<&str, &McpError>,
                ) -> Result<(), HostError> {
                    Err(unavailable("respond"))
                }

                pub fn notify(
                    _listener: &str,
                    _connection: Option<u64>,
                    _method: &str,
                    _params: &str,
                ) -> Result<(), HostError> {
                    Err(unavailable("notify"))
                }

                pub fn send(
                    _listener: &str,
                    _connection: u64,
                    _text: &str,
                ) -> Result<(), HostError> {
                    Err(unavailable("send"))
                }

                pub fn reply(_request_id: u64, _status: u16, _body: &str) -> Result<(), HostError> {
                    Err(unavailable("reply"))
                }
            }
        };
    }

    older!(v0_2, "0.2");
    older!(v0_1, "0.1");
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::{HostEffect, take_host_effects};

    #[test]
    fn listener_calls_are_recorded_off_wasm() {
        take_host_effects();
        let tool = ToolDef::new("echo", "Echoes its text.", r#"{"type":"object"}"#);
        register_tools("ide", std::slice::from_ref(&tool)).expect("register");
        respond(7, Ok(r#"{"content":[]}"#)).expect("respond");
        let error = McpError::new(INVALID_PARAMS, "no text");
        respond(8, Err(&error)).expect("respond");
        notify("ide", None, "selection_changed", "{}").expect("notify");
        send("raw", 3, "hi").expect("send");
        reply(9, 200, "ok").expect("reply");
        assert_eq!(
            take_host_effects(),
            [
                HostEffect::Listener(ListenerEffect::RegisterTools {
                    listener: "ide".into(),
                    tools: vec![tool],
                }),
                HostEffect::Listener(ListenerEffect::Respond {
                    call_id: 7,
                    outcome: Ok(r#"{"content":[]}"#.into()),
                }),
                HostEffect::Listener(ListenerEffect::Respond {
                    call_id: 8,
                    outcome: Err(error),
                }),
                HostEffect::Listener(ListenerEffect::Notify {
                    listener: "ide".into(),
                    connection: None,
                    method: "selection_changed".into(),
                    params: "{}".into(),
                }),
                HostEffect::Listener(ListenerEffect::Send {
                    listener: "raw".into(),
                    connection: 3,
                    text: "hi".into(),
                }),
                HostEffect::Listener(ListenerEffect::Reply {
                    request_id: 9,
                    status: 200,
                    body: "ok".into(),
                }),
            ]
        );
    }

    #[test]
    fn older_worlds_have_no_listeners() {
        let error = glue::v0_2::register_tools("ide", &[]).unwrap_err();
        assert_eq!(error.code, crate::HostErrorCode::Unavailable);
        assert!(
            error.message.contains("register_tools needs fyx:ui 0.4"),
            "{error:?}"
        );
        assert!(glue::v0_1::respond(1, Ok("{}")).is_err());
    }
}
