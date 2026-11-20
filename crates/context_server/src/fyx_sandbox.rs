//! fyx: plan 7.5 hook. Fyx can confine a stdio MCP server before it starts (on macOS, under a
//! Seatbelt profile). The policy lives in a Fyx crate (`ui_ext_host::mcp_sandbox`), which installs
//! a [`StdioCommandWrapper`]; without one, servers start as in Zed.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, PoisonError, RwLock};

use anyhow::Result;

/// The server a command starts.
#[derive(Clone, Copy, Debug)]
pub struct StdioServer<'a> {
    /// The server's id: its key under `context_servers` in the settings.
    pub id: &'a str,
    pub working_directory: Option<&'a Path>,
}

/// Takes the command that would start an MCP server and returns the one to run instead. An error
/// refuses to start the server.
pub type StdioCommandWrapper =
    Arc<dyn Fn(Command, StdioServer<'_>) -> Result<Command> + Send + Sync + 'static>;

static WRAPPER: RwLock<Option<StdioCommandWrapper>> = RwLock::new(None);

/// Installs (or, with `None`, removes) the wrapper every later stdio server start goes through.
pub fn set_stdio_command_wrapper(wrapper: Option<StdioCommandWrapper>) {
    *WRAPPER.write().unwrap_or_else(PoisonError::into_inner) = wrapper;
}

pub(crate) fn wrap(command: Command, server: StdioServer<'_>) -> Result<Command> {
    let wrapper = WRAPPER
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    match wrapper {
        Some(wrapper) => wrapper(command, server),
        None => Ok(command),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrapper_replaces_the_command_and_an_error_refuses_it() {
        let command = || {
            let mut command = Command::new("server");
            command.arg("--stdio");
            command
        };
        let server = StdioServer {
            id: "files",
            working_directory: Some(Path::new("project")),
        };
        let unwrapped = wrap(command(), server).expect("no wrapper starts the command as is");
        assert_eq!(unwrapped.get_program(), "server");

        set_stdio_command_wrapper(Some(Arc::new(|inner, server| {
            let mut outer = Command::new("sandbox");
            outer
                .arg(server.id)
                .arg(inner.get_program())
                .args(inner.get_args());
            if let Some(working_directory) = server.working_directory {
                outer.current_dir(working_directory);
            }
            Ok(outer)
        })));
        let wrapped = wrap(command(), server).expect("the wrapper succeeds");
        assert_eq!(wrapped.get_program(), "sandbox");
        assert_eq!(
            wrapped.get_args().collect::<Vec<_>>(),
            ["files", "server", "--stdio"]
        );
        assert_eq!(wrapped.get_current_dir(), Some(Path::new("project")));

        set_stdio_command_wrapper(Some(Arc::new(|_, _| anyhow::bail!("not allowed"))));
        assert!(wrap(command(), server).is_err());

        set_stdio_command_wrapper(None);
        assert!(wrap(command(), server).is_ok());
    }
}
